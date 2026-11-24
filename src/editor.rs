use crate::theme::Palette;
use crate::{config::Config, state};
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind,
        KeyModifiers,
    },
    execute, queue,
    style::{
        Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
    },
    terminal::{
        self, Clear, ClearType, DisableLineWrap, EnableLineWrap, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};
use serde_json::Value;
use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

// Tabs begin at these fields. Up/Down also traverses section boundaries.
const SECTIONS: &[(&str, &str)] = &[
    ("Agent", "agentKind"),
    ("Sandbox", "workspaceMode"),
    ("Runtime", "cpus"),
    ("Network", "publish"),
    ("Herdr", "openIn"),
];
const FIELDS: &[(&str, &str, &str)] = &[
    (
        "agentKind",
        "Coding agent",
        "Choose the agent to launch in new sandboxes.",
    ),
    (
        "customAgents",
        "Custom profiles",
        "JSON object mapping agent names to profiles.",
    ),
    (
        "agentArgs",
        "Launch arguments",
        "JSON object mapping agent names to argument lists.",
    ),
    (
        "agentEnv",
        "Agent environment",
        "JSON list of environment variables for the agent.",
    ),
    (
        "workspaceMode",
        "Workspace mode",
        "Mount the host checkout or work in a separate clone.",
    ),
    (
        "extraWorkspaces",
        "Extra workspaces",
        "JSON list of additional workspace paths.",
    ),
    ("template", "Template", "Sandbox template used at creation."),
    ("kits", "Kits", "JSON list of kits to install."),
    (
        "kitArgs",
        "Kit arguments",
        "JSON list of arguments to pass to kits.",
    ),
    (
        "env",
        "Environment",
        "JSON list of sandbox environment variables.",
    ),
    (
        "envFiles",
        "Environment files",
        "JSON list of paths to environment files.",
    ),
    (
        "cpus",
        "CPU count",
        "Number of CPUs allocated to the sandbox.",
    ),
    (
        "memory",
        "Memory limit",
        "Memory allocated to the sandbox, for example 4g.",
    ),
    ("shell", "Login shell", "Shell to open inside the sandbox."),
    (
        "sbxBin",
        "CLI location",
        "Path to the Docker Sandboxes executable on the host.",
    ),
    (
        "sandboxNamePrefix",
        "Name prefix",
        "Prefix used when naming a new sandbox.",
    ),
    (
        "publish",
        "Published ports",
        "JSON list of port mappings to publish.",
    ),
    (
        "denyNetwork",
        "Denied destinations",
        "JSON list of network destinations to block.",
    ),
    (
        "openIn",
        "Open agent in",
        "Choose a split pane or a new tab.",
    ),
    (
        "paneDirection",
        "Split direction",
        "Direction for new split panes.",
    ),
    (
        "paneRatio",
        "Split ratio",
        "Share of the pane given to the split, between 0 and 1.",
    ),
    (
        "reportAgentStatus",
        "Report agent status",
        "Send agent status updates to Herdr.",
    ),
    (
        "cleanupOnWorktreeRemoved",
        "Worktree cleanup",
        "Offer to remove sandboxes when their worktree is removed.",
    ),
];

fn section_start(index: usize) -> usize {
    SECTIONS
        .get(index)
        .and_then(|(_, key)| FIELDS.iter().position(|f| f.0 == *key))
        .unwrap_or(FIELDS.len())
}
fn section_of(selected: usize) -> usize {
    (1..SECTIONS.len())
        .take_while(|&i| section_start(i) <= selected)
        .count()
}

fn choices(key: &str, raw: &Value) -> Vec<String> {
    if key == "agentKind" {
        let mut names: Vec<_> = Config::builtins()
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        if let Some(custom) = raw["customAgents"].as_object() {
            names.extend(custom.keys().cloned());
        }
        names.sort();
        names.dedup();
        return names;
    }
    let options: &[&str] = match key {
        "workspaceMode" => &["mount", "clone"],
        "openIn" => &["split", "tab"],
        "paneDirection" => &["right", "down"],
        "reportAgentStatus" | "cleanupOnWorktreeRemoved" => &["true", "false"],
        _ => &[],
    };
    options.iter().map(|s| s.to_string()).collect()
}
fn formatted(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        _ => value.to_string(),
    }
}
fn typed(key: &str, input: &str, defaults: &Value) -> Result<Value, String> {
    if input.is_empty() {
        return Ok(defaults[key].clone());
    }
    if defaults[key].is_string() || matches!(key, "template" | "memory" | "sbxBin") {
        Ok(Value::String(input.into()))
    } else {
        serde_json::from_str(input).map_err(|e| e.to_string())
    }
}

struct Form {
    path: PathBuf,
    original: Option<String>,
    initial: Value,
    raw: Value,
    defaults: Value,
    selected: usize,
    edit: Option<Vec<char>>,
    picker: Option<usize>,
    cursor: usize,
    message: String,
}
impl Form {
    fn open(path: PathBuf) -> Result<Self, String> {
        let original = Config::read(&path)?;
        let raw = Config::parse(original.as_deref())?;
        if !raw.is_object() {
            return Err("config.json must contain a JSON object.".into());
        }
        Ok(Self {
            path,
            original,
            initial: raw.clone(),
            raw,
            defaults: Config::defaults(),
            selected: 0,
            edit: None,
            picker: None,
            cursor: 0,
            message: String::new(),
        })
    }
    fn key(&self) -> &'static str {
        FIELDS[self.selected].0
    }
    fn value(&self) -> &Value {
        self.raw
            .get(self.key())
            .unwrap_or(&self.defaults[self.key()])
    }
    fn apply(&mut self, input: &str) -> Result<(), String> {
        let key = self.key();
        let v = typed(key, input, &self.defaults)?;
        if v == self.defaults[key] {
            self.raw.as_object_mut().unwrap().remove(key);
        } else {
            self.raw[key] = v;
        }
        Ok(())
    }
    fn save(&self) -> Result<(), String> {
        let cfg = Config::from_raw(&self.raw)?;
        cfg.agent(cfg.values["agentKind"].as_str().unwrap())?;
        if self.raw == self.initial {
            return Ok(());
        }
        let dir = self.path.parent().unwrap();
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let _lock = state::lock(self.path.with_extension("json.lock")).map_err(|e| e.message)?;
        if Config::read(&self.path)? != self.original {
            return Err("Config changed on disk. Esc to close, then reopen.".into());
        }
        let write = || -> io::Result<()> {
            let mut file = tempfile::NamedTempFile::new_in(dir)?;
            if let Ok(meta) = fs::metadata(&self.path) {
                file.as_file().set_permissions(meta.permissions())?;
            }
            serde_json::to_writer_pretty(&mut file, &self.raw)?;
            writeln!(file)?;
            file.flush()?;
            file.persist(&self.path)?;
            Ok(())
        };
        write().map_err(|e| e.to_string())
    }
    fn input(&mut self, code: KeyCode, modifiers: KeyModifiers) -> bool {
        let ctrl = modifiers.contains(KeyModifiers::CONTROL);
        self.message.clear();
        if ctrl && code == KeyCode::Char('c') {
            return true;
        }
        if let Some(i) = self.picker {
            let opts = choices(self.key(), &self.raw);
            self.picker = match code {
                KeyCode::Esc => None,
                KeyCode::Enter => {
                    if let Err(e) = self.apply(&opts[i]) {
                        self.message = e;
                    }
                    None
                }
                KeyCode::Down | KeyCode::Right => Some((i + 1) % opts.len()),
                KeyCode::Up | KeyCode::Left => Some((i + opts.len() - 1) % opts.len()),
                _ => Some(i),
            };
            return false;
        }
        if let Some(buf) = &mut self.edit {
            match code {
                KeyCode::Esc => self.edit = None,
                KeyCode::Enter => {
                    let text = buf.iter().collect::<String>();
                    match self.apply(&text) {
                        Ok(()) => self.edit = None,
                        Err(e) => self.message = e,
                    }
                }
                KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
                KeyCode::Right => self.cursor = (self.cursor + 1).min(buf.len()),
                KeyCode::Home => self.cursor = 0,
                KeyCode::End => self.cursor = buf.len(),
                KeyCode::Backspace if self.cursor > 0 => {
                    self.cursor -= 1;
                    buf.remove(self.cursor);
                }
                KeyCode::Delete if self.cursor < buf.len() => {
                    buf.remove(self.cursor);
                }
                KeyCode::Char('u') if ctrl => {
                    buf.clear();
                    self.cursor = 0;
                }
                KeyCode::Char(c) if !ctrl && !c.is_control() => {
                    buf.insert(self.cursor, c);
                    self.cursor += 1;
                }
                _ => {}
            }
        } else {
            match code {
                KeyCode::Esc | KeyCode::Char('q') => return true,
                KeyCode::Char('s' | 'S') => match self.save() {
                    Ok(()) => return true,
                    Err(e) => self.message = e,
                },
                KeyCode::Tab | KeyCode::BackTab => {
                    let i = section_of(self.selected);
                    let step = if code == KeyCode::BackTab {
                        SECTIONS.len() - 1
                    } else {
                        1
                    };
                    self.selected = section_start((i + step) % SECTIONS.len());
                }
                KeyCode::Down => self.selected = (self.selected + 1) % FIELDS.len(),
                KeyCode::Up => self.selected = (self.selected + FIELDS.len() - 1) % FIELDS.len(),
                KeyCode::Delete => {
                    let key = self.key();
                    self.raw.as_object_mut().unwrap().remove(key);
                }
                KeyCode::Left | KeyCode::Right | KeyCode::Enter => {
                    let opts = choices(self.key(), &self.raw);
                    if !opts.is_empty() {
                        let current = formatted(self.value());
                        if code == KeyCode::Enter {
                            self.picker =
                                Some(opts.iter().position(|s| s == &current).unwrap_or(0));
                            return false;
                        }
                        let next = opts.iter().position(|s| s == &current).map_or(0, |i| {
                            if code == KeyCode::Left {
                                (i + opts.len() - 1) % opts.len()
                            } else {
                                (i + 1) % opts.len()
                            }
                        });
                        if let Err(e) = self.apply(&opts[next]) {
                            self.message = e;
                        }
                    } else if code == KeyCode::Enter {
                        let buf: Vec<_> = formatted(self.value()).chars().collect();
                        self.cursor = buf.len();
                        self.edit = Some(buf);
                    }
                }
                _ => {}
            }
        }
        false
    }
    fn option_title<'a>(&'a self, key: &str, value: &'a str) -> &'a str {
        match (key, value) {
            ("agentKind", k) => self.raw["customAgents"][k]["title"]
                .as_str()
                .or_else(|| Config::builtins()[k]["title"].as_str())
                .unwrap_or(k),
            (_, "true") => "On",
            (_, "false") => "Off",
            _ => value,
        }
    }
    fn summary(&self, key: &str) -> String {
        let v = self.raw.get(key).unwrap_or(&self.defaults[key]);
        match v {
            Value::Array(a) if a.is_empty() => "None".into(),
            Value::Object(a) if a.is_empty() => "None".into(),
            Value::Array(a) => format!("{} entries", a.len()),
            Value::Object(o) => format!("{} entries", o.len()),
            Value::Null => "Automatic".into(),
            Value::Bool(b) => if *b { "On" } else { "Off" }.into(),
            Value::String(k) if key == "agentKind" => self.option_title(key, k).into(),
            _ => formatted(v),
        }
    }
    fn draw<W: Write>(&self, out: &mut W, p: &Palette, (w, h): (u16, u16)) -> io::Result<()> {
        queue!(
            out,
            Hide,
            SetBackgroundColor(p.panel),
            Clear(ClearType::All)
        )?;
        let put =
            |out: &mut W, x: u16, y: u16, fg: Color, bg: Color, text: &str| -> io::Result<()> {
                let safe: String = text
                    .chars()
                    .take(w.saturating_sub(x) as usize)
                    .map(|c| if c.is_control() { ' ' } else { c })
                    .collect();
                queue!(
                    out,
                    MoveTo(x, y),
                    SetForegroundColor(fg),
                    SetBackgroundColor(bg),
                    Print(safe)
                )
            };
        if w < 26 || h < 10 {
            put(out, 0, 0, p.text, p.panel, "Enlarge settings popup")?;
            return out.flush();
        }
        let section = section_of(self.selected);
        let start = section_start(section);
        let end = section_start(section + 1);
        let (key, _, help) = FIELDS[self.selected];
        let compact = w < 48;
        let pad = if compact { 1 } else { 2 };
        queue!(out, SetAttribute(Attribute::Bold))?;
        put(out, pad, 0, p.text, p.panel, "Sandbox settings")?;
        queue!(out, SetAttribute(Attribute::NormalIntensity))?;
        if self.raw != self.initial && w >= 40 {
            put(out, w - 10, 0, p.accent, p.panel, "Modified")?;
        }

        let mut x = pad;
        for (i, (name, _)) in SECTIONS.iter().enumerate() {
            if compact && i != section {
                continue;
            }
            let (fg, bg) = if i == section {
                (p.on_accent, p.accent)
            } else {
                (p.muted, p.panel)
            };
            let label = format!(" {name} ");
            put(out, x, 2, fg, bg, &label)?;
            x += label.len() as u16 + 1;
        }
        let rule = "─".repeat(w.saturating_sub(2 * pad) as usize);
        put(out, pad, 3, p.rule, p.panel, &rule)?;
        let rows = usize::from(h.saturating_sub(10).max(1));
        let column = if compact { w - w / 3 } else { (w / 2).min(36) };
        let opts = choices(key, &self.raw);
        let selected = self.picker.unwrap_or(self.selected);
        let (start, end) = if self.picker.is_some() {
            (0, opts.len())
        } else {
            (start, end)
        };
        let first = start.max((selected + 1).saturating_sub(rows));
        for (row, i) in (first..end).take(rows).enumerate() {
            let (label, value, dim) = if self.picker.is_some() {
                let option = &opts[i];
                let current = *option == formatted(self.value());
                (
                    self.option_title(key, option).to_owned(),
                    if current {
                        "✓ Current".into()
                    } else {
                        String::new()
                    },
                    !current,
                )
            } else {
                let (field, label, _) = FIELDS[i];
                (
                    label.to_owned(),
                    self.summary(field),
                    self.raw.get(field).is_none(),
                )
            };
            let y = 4 + row as u16;
            let active = i == selected;
            let (fg, bg) = if active {
                (p.on_accent, p.accent)
            } else {
                (p.text, p.panel)
            };
            if active {
                put(out, pad, y, fg, bg, &" ".repeat((w - 2 * pad) as usize))?;
            }
            let marker = if active { "▸" } else { " " };
            let title = format!("{marker} {label}");
            put(
                out,
                pad,
                y,
                fg,
                bg,
                &title
                    .chars()
                    .take((column - pad - 1) as usize)
                    .collect::<String>(),
            )?;
            let color = if !active && dim { p.muted } else { fg };
            put(
                out,
                column,
                y,
                color,
                bg,
                &value
                    .chars()
                    .take((w - column - pad) as usize)
                    .collect::<String>(),
            )?;
        }
        put(out, pad, h - 5, p.rule, p.panel, &rule)?;
        put(out, pad, h - 4, p.accent, p.panel, key)?;
        let hint = if self.picker.is_some() {
            "↑↓ select   Enter apply   Esc back"
        } else if self.edit.is_some() {
            "Enter apply   Esc undo   Ctrl+U clear"
        } else if compact {
            "Tab section  S save  Esc close"
        } else {
            "Tab section   ↑↓ select   Del default   S save   Esc close"
        };
        put(out, pad, h - 1, p.muted, p.panel, hint)?;
        if !self.message.is_empty() {
            put(out, pad, h - 2, p.error, p.panel, &self.message)?;
        } else if self.edit.is_none() && self.picker.is_none() {
            let instruction = if choices(key, &self.raw).is_empty() {
                "Enter to edit"
            } else {
                "Enter to choose   ← → to cycle"
            };
            put(out, pad, h - 2, p.accent, p.panel, instruction)?;
        }
        if let Some(buf) = &self.edit {
            let width = (w - 2 * pad) as usize;
            let offset = (self.cursor + 1).saturating_sub(width);
            let y = h - 3;
            put(out, pad, y, p.text, p.rule, &" ".repeat(width))?;
            let before: String = buf[offset..self.cursor].iter().collect();
            put(out, pad, y, p.text, p.rule, &before)?;
            queue!(out, crossterm::cursor::SavePosition)?;
            let after: String = buf[self.cursor..]
                .iter()
                .take(width - before.chars().count())
                .map(|c| if c.is_control() { ' ' } else { *c })
                .collect();
            queue!(out, Print(after), crossterm::cursor::RestorePosition, Show)?;
        } else {
            put(out, pad, h - 3, p.text, p.panel, help)?;
        }
        out.flush()
    }
}

struct Screen;
impl Drop for Screen {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), DisableBracketedPaste);
        let _ = execute!(
            io::stdout(),
            ResetColor,
            EnableLineWrap,
            Show,
            LeaveAlternateScreen
        );
        let _ = terminal::disable_raw_mode();
    }
}
pub fn run() -> Result<(), String> {
    let dir = env::var_os("HERDR_PLUGIN_CONFIG_DIR")
        .filter(|s| !s.is_empty())
        .ok_or("Missing plugin config directory.")?;
    let mut form = Form::open(Path::new(&dir).join("config.json"))?;
    let palette = Palette::load();
    let mut ui = || -> io::Result<()> {
        terminal::enable_raw_mode()?;
        let _screen = Screen;
        let mut out = io::stdout();
        execute!(out, EnterAlternateScreen, DisableLineWrap)?;
        let _ = execute!(out, EnableBracketedPaste);
        loop {
            form.draw(&mut out, &palette, terminal::size()?)?;
            match event::read()? {
                Event::Key(k) if k.kind != KeyEventKind::Release => {
                    if form.input(k.code, k.modifiers) {
                        break;
                    }
                }
                Event::Paste(text) => {
                    if let Some(buf) = &mut form.edit {
                        let chars: Vec<_> = text
                            .chars()
                            .filter_map(|c| match c {
                                '\n' | '\r' | '\t' => Some(' '),
                                c if c.is_control() => None,
                                c => Some(c),
                            })
                            .collect();
                        let n = chars.len();
                        buf.splice(form.cursor..form.cursor, chars);
                        form.cursor += n;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    };
    ui().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn form_covers_config_and_saves_only_valid_explicit_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut form = Form::open(path.clone()).unwrap();
        let mut keys: Vec<_> = FIELDS.iter().map(|(key, _, _)| *key).collect();
        keys.sort();
        assert_eq!(
            keys,
            form.defaults
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>()
        );
        form.save().unwrap();
        assert!(!path.exists());
        form.input(KeyCode::Right, KeyModifiers::NONE);
        let selected = form.raw["agentKind"].clone();
        assert!(selected.is_string());
        form.selected = FIELDS.iter().position(|f| f.0 == "paneRatio").unwrap();
        form.apply("2").unwrap();
        assert!(form.save().is_err());
        assert!(!path.exists());
        form.apply("0.3").unwrap();
        form.save().unwrap();
        let raw = Form::open(path).unwrap().raw;
        assert_eq!(
            raw,
            serde_json::json!({"agentKind":selected,"paneRatio":0.3})
        );
    }
    #[test]
    fn escape_discards_input_and_another_editor_cannot_overwrite_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut first = Form::open(path.clone()).unwrap();
        let mut second = Form::open(path.clone()).unwrap();
        first.selected = FIELDS.iter().position(|f| f.0 == "shell").unwrap();
        for key in [
            KeyCode::Enter,
            KeyCode::End,
            KeyCode::Char('x'),
            KeyCode::Esc,
        ] {
            first.input(key, KeyModifiers::NONE);
        }
        assert_eq!(first.raw, serde_json::json!({}));
        first.apply("zsh").unwrap();
        first.save().unwrap();
        second.input(KeyCode::Right, KeyModifiers::NONE);
        assert!(second.save().unwrap_err().contains("changed on disk"));
        assert_eq!(
            Form::open(path).unwrap().raw,
            serde_json::json!({"shell":"zsh"})
        );
    }
}
