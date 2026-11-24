use crate::{
    config::Config,
    inventory,
    location::Location,
    management, sbx,
    screen::{wrap, Frame, Screen},
    theme::Palette,
};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    terminal,
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    env,
    io::{self, IsTerminal, Read},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

const REFRESH: Duration = Duration::from_secs(3);
type PortList = Vec<(Option<u16>, u16)>;

fn clean(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') {
                '?'
            } else {
                c
            }
        })
        .collect()
}

struct Row {
    snapshot: Value,
    location: Location,
    pane: String,
    name: String,
    status: String,
    closed: Option<bool>,
    exists: Option<bool>,
    tracked_exists: Option<bool>,
}

impl Row {
    fn state(&self) -> &'static str {
        match (self.closed, self.tracked_exists) {
            (Some(true), Some(false)) => "Stale mapping",
            (Some(true), Some(true)) => "Pane closed",
            (Some(false), Some(false)) => "Sandbox gone",
            (Some(false), Some(true)) => "Pane open",
            _ => "Unverified",
        }
    }
    fn action(&self, key: char) -> Option<&'static str> {
        match (key, self.closed?, self.tracked_exists?) {
            ('d', true, false) => Some("forget-missing"),
            ('d', _, true) => Some("forget-mapping"),
            ('s', _, true) if self.exists == Some(true) && self.status != "stopped" => Some("stop"),
            ('r', _, true) if self.exists == Some(true) => Some("reconnect"),
            ('d', false, false) => Some("forget-mapping"),
            _ => None,
        }
    }
}

struct Overlay {
    state: PathBuf,
    sbx: String,
    herdr: String,
    stop: Arc<AtomicBool>,
    rows: Vec<Row>,
    warnings: Vec<String>,
    cache: BTreeMap<String, Result<PortList, String>>,
}

impl Overlay {
    fn new(stop: Arc<AtomicBool>) -> Result<Self, String> {
        let dir = |key| {
            env::var_os(key)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .ok_or_else(|| format!("{key} is not set. Run through Herdr."))
        };
        Ok(Self {
            state: dir("HERDR_PLUGIN_STATE_DIR")?,
            sbx: Config::load(&dir("HERDR_PLUGIN_CONFIG_DIR")?)?.sbx,
            herdr: env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into()),
            stop,
            rows: vec![],
            warnings: vec![],
            cache: BTreeMap::new(),
        })
    }

    fn query(&self, bin: &str, args: &[&str], seconds: u64) -> Result<Value, String> {
        let output = sbx::run_cancellable(bin, args, Duration::from_secs(seconds), || {
            self.stop.load(Ordering::Relaxed)
        })
        .map_err(|e| {
            if e.kind == "startup" {
                format!("Could not run {bin}")
            } else {
                e.message
            }
        })?;
        if !output.status.success() {
            return Err(format!("{}: {}", output.status, output.combined.trim()));
        }
        serde_json::from_str(&output.stdout).map_err(|e| format!("invalid JSON: {e}"))
    }

    fn frame(&mut self, refresh_ports: bool, links: bool, once: bool) -> Result<String, String> {
        self.rows.clear();
        self.warnings.clear();
        let rows = inventory::mappings(&self.state)?;
        let live = self
            .query(&self.sbx, &["ls", "--json"], 15)
            .and_then(|v| inventory::sandboxes(&v));
        let response = self.query(&self.herdr, &["api", "snapshot"], 10);
        let snapshot = response
            .as_ref()
            .ok()
            .and_then(|v| v.pointer("/result/snapshot"))
            .unwrap_or(&Value::Null);
        let panes = response
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|_| inventory::pane_ids(snapshot));
        let mut lines = vec![
            format!(
                "Docker Sandboxes  {} mapping{}  {} UTC  (q closes, refreshes every 3s)",
                rows.len(),
                if rows.len() == 1 { "" } else { "s" },
                chrono::DateTime::<chrono::Utc>::from(std::time::SystemTime::now())
                    .format("%H:%M:%S")
            ),
            String::new(),
        ];
        if rows.is_empty() {
            lines.push("No sandboxes are mapped. Run start-agent from a workspace.".into());
        } else {
            lines.push(format!(
                "{:<14} {:<30} {:<12} {:<12} {:<10} {:<18} DIRECTORY",
                "PANE", "SANDBOX", "AGENT", "STATE", "STATUS", "PORTS"
            ));
        }
        let mut stale = 0;
        let mut tracked = std::collections::HashSet::new();
        for (id, row) in &rows {
            if self.stop.load(Ordering::Relaxed) {
                break;
            }
            let name = row["sandboxName"].as_str().unwrap_or("");
            let exists = live.as_ref().ok().map(|m| m.contains_key(name));
            let pane = clean(id);
            let pane = match &panes {
                Ok(ids) if !ids.contains(id) => {
                    if exists == Some(false) {
                        stale += 1;
                    }
                    format!("{pane} (closed)")
                }
                Err(_) => format!("{pane} (?)"),
                _ => pane,
            };
            if exists == Some(false) {
                self.cache.remove(name);
            }
            if tracked.insert(name.to_string())
                && exists == Some(true)
                && (refresh_ports || !self.cache.contains_key(name))
            {
                let result = crate::ports::query(name, |args| {
                    sbx::run_cancellable(&self.sbx, args, Duration::from_secs(15), || {
                        self.stop.load(Ordering::Relaxed)
                    })
                })
                .map_err(|e| e.message)
                .and_then(|v| inventory::ports(&v));
                self.cache.insert(name.to_string(), result);
            }
            let status = match &live {
                Ok(m) => m
                    .get(name)
                    .map(|s| s.as_str().unwrap_or("exists"))
                    .unwrap_or("MISSING"),
                Err(_) => "unknown",
            };
            let ports = port_cell(name, self.cache.get(name), links);
            let field = |key: &str| clean(row[key].as_str().unwrap_or(""));
            let tracked_exists = management::names(row).ok().and_then(|names| {
                live.as_ref()
                    .ok()
                    .map(|items| names.iter().any(|n| items.contains_key(*n)))
            });
            self.rows.push(Row {
                snapshot: row.clone(),
                location: Location::resolve(snapshot, id, row["workspaceId"].as_str()),
                pane: id.clone(),
                name: name.into(),
                status: clean(status),
                closed: panes.as_ref().ok().map(|ids| !ids.contains(id)),
                exists,
                tracked_exists,
            });
            if once {
                lines.push(format!(
                    "{pane:<14} {:<30} {:<12} {:<12} {:<10} {ports} {}",
                    clean(name),
                    field("agentKind"),
                    field("lifecycleState"),
                    clean(status),
                    field("localPath")
                ));
            }
        }
        self.cache.retain(|name, _| tracked.contains(name));
        if stale > 0 {
            lines.push(format!("{stale} stale mapping(s) (pane gone, sandbox missing): run prune-mappings to drop them"));
        }
        if let Err(e) = panes {
            self.warnings
                .push(format!("pane check failed: {}", clean(&e)));
        }
        if let Err(e) = live {
            self.warnings.push(format!("sbx ls failed: {}", clean(&e)));
        }
        lines.extend(self.warnings.iter().cloned());
        Ok(lines.join("\n"))
    }
}

fn port_cell(name: &str, ports: Option<&Result<PortList, String>>, links: bool) -> String {
    let mut plain = Vec::new();
    let mut rendered = Vec::new();
    if let Some(Ok(list)) = ports {
        for (host, guest) in list {
            let label = format!(
                "{}->{guest}",
                host.map_or_else(|| "?".into(), |h| h.to_string())
            );
            rendered.push(if links {
                crate::ports::terminal_link(name, *guest, *host, &label, cfg!(windows))
            } else {
                label.clone()
            });
            plain.push(label);
        }
    }
    if plain.is_empty() {
        return format!(
            "{:<18}",
            if matches!(ports, Some(Err(_))) {
                "error"
            } else {
                "-"
            }
        );
    }
    let pad = 18usize.saturating_sub(plain.join(" ").len());
    format!("{}{}", rendered.join(" "), " ".repeat(pad))
}

struct Raw(bool);
impl Drop for Raw {
    fn drop(&mut self) {
        if self.0 {
            let _ = terminal::disable_raw_mode();
        }
    }
}

fn input(stop: Arc<AtomicBool>, tx: Sender<char>, any_key: bool) -> Raw {
    let raw = io::stdin().is_terminal() && terminal::enable_raw_mode().is_ok();
    thread::spawn(move || {
        if raw {
            while let Ok(e) = event::read() {
                if let Event::Key(key) = e {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }
                    if any_key
                        || matches!(key.code, KeyCode::Char('q' | 'Q'))
                        || (key.code == KeyCode::Char('c')
                            && key.modifiers.contains(KeyModifiers::CONTROL))
                    {
                        break;
                    }
                    let cmd = match key.code {
                        KeyCode::Up => 'k',
                        KeyCode::Down => 'j',
                        KeyCode::Char(c @ ('j' | 'k' | 'r' | 's' | 'd' | 'i')) => c,
                        KeyCode::Esc => break,
                        _ => continue,
                    };
                    if tx.send(cmd).is_err() {
                        break;
                    }
                }
            }
        } else {
            for b in io::stdin().lock().bytes().map_while(Result::ok) {
                if any_key || matches!(b, b'q' | b'Q' | 3) {
                    break;
                }
                if b"jkrsdi".contains(&b) && tx.send(char::from(b)).is_err() {
                    break;
                }
            }
        }
        stop.store(true, Ordering::Relaxed);
    });
    Raw(raw)
}

fn wait(stop: &AtomicBool, duration: Duration) {
    let start = Instant::now();
    while !stop.load(Ordering::Relaxed) && start.elapsed() < duration {
        thread::sleep(Duration::from_millis(25));
    }
}

fn inspector(
    frame: &mut Frame,
    p: &Palette,
    row: &Row,
    overlay: &Overlay,
    x: u16,
    end: u16,
) -> io::Result<()> {
    let width = frame.w.saturating_sub(x + 1);
    let mut y = 3;
    for (label, value) in [
        ("Sandbox", row.name.as_str()),
        ("Agent", row.snapshot["agentKind"].as_str().unwrap_or("—")),
        ("Workspace", row.location.workspace.as_str()),
        ("Pane", row.location.pane.as_str()),
        ("Mapping", row.state()),
        ("Docker", row.status.as_str()),
        (
            "Worktree",
            row.snapshot["localPath"].as_str().unwrap_or("—"),
        ),
    ] {
        if y >= end {
            return Ok(());
        }
        let indent = 11;
        frame.label(x, y, width, p.muted, label)?;
        for part in wrap(&clean(value), usize::from(width.saturating_sub(indent))) {
            if y >= end {
                return Ok(());
            }
            frame.label(x + indent, y, width.saturating_sub(indent), p.text, &part)?;
            y += 1;
        }
    }
    if y < end {
        frame.label(x, y, width, p.muted, "Ports")?;
        match overlay.cache.get(&row.name) {
            Some(Ok(ports)) if !ports.is_empty() => {
                for (host, guest) in ports {
                    if y >= end {
                        break;
                    }
                    let label = format!(
                        "{} → {guest}",
                        host.map_or_else(|| "?".into(), |n| n.to_string())
                    );
                    if label.chars().count() <= usize::from(width.saturating_sub(11)) {
                        let link = crate::ports::terminal_link(
                            &row.name,
                            *guest,
                            *host,
                            &label,
                            cfg!(windows),
                        );
                        frame.link(x + 11, y, &link)?;
                    } else {
                        frame.label(x + 11, y, width.saturating_sub(11), p.text, &label)?;
                    }
                    y += 1;
                }
            }
            result => frame.label(
                x + 11,
                y,
                width.saturating_sub(11),
                p.text,
                if matches!(result, Some(Err(_))) {
                    "Unavailable"
                } else {
                    "—"
                },
            )?,
        }
    }
    Ok(())
}

fn draw(
    overlay: &Overlay,
    pos: Option<usize>,
    details: bool,
    status: &str,
    p: &Palette,
) -> io::Result<()> {
    let size = if io::stdout().is_terminal() {
        terminal::size().unwrap_or((100, 24))
    } else {
        (100, 24)
    };
    let mut f = Frame::new(p, size)?;
    let (w, h) = size;
    let wide = w >= 76;
    let show_list = wide || !details;
    let row = pos.map(|i| &overlay.rows[i]);
    let mut commands = vec!["↑↓ select"];
    if !wide {
        commands.push(if details { "i list" } else { "i details" });
    }
    if let Some(row) = row {
        for (key, hint) in [('r', "r reconnect"), ('s', "s stop"), ('d', "d delete…")] {
            if let Some(action) = row.action(key) {
                commands.push(if action == "forget-missing" {
                    "d remove stale mapping"
                } else {
                    hint
                });
            }
        }
    }
    commands.push("q close");
    let mut footer = vec![String::new()];
    let footer_width = usize::from(w.saturating_sub(2));
    for command in commands {
        let line = footer.last_mut().unwrap();
        if !line.is_empty() && line.chars().count() + 3 + command.chars().count() > footer_width {
            footer.push(command.to_owned());
        } else {
            if !line.is_empty() {
                line.push_str("   ");
            }
            line.push_str(command);
        }
    }
    let footer_y = h.saturating_sub(footer.len() as u16);
    let status_y = footer_y.saturating_sub(1);
    let list_w = if wide {
        ((u32::from(w) * 3 / 5) as u16).min(w - 32)
    } else {
        w
    };
    f.label(
        1,
        0,
        w,
        p.text,
        &format!("Docker Sandboxes   ·   {} tracked", overlay.rows.len()),
    )?;
    if show_list {
        let name_w = list_w.saturating_sub(26).max(6);
        let docker_x = name_w + 3;
        let map_x = docker_x + 9;
        f.label(2, 2, name_w, p.muted, "Sandbox")?;
        f.label(docker_x, 2, 8, p.muted, "Docker")?;
        f.label(map_x, 2, list_w.saturating_sub(map_x), p.muted, "Mapping")?;
        let visible = usize::from(status_y.saturating_sub(3));
        let first = (pos.unwrap_or(0) + 1).saturating_sub(visible);
        for (n, row) in overlay.rows.iter().enumerate().skip(first).take(visible) {
            let y = 3 + (n - first) as u16;
            let (fg, bg) = if pos == Some(n) {
                (p.on_accent, p.accent)
            } else {
                (p.text, p.panel)
            };
            if pos == Some(n) {
                f.text(
                    0,
                    y,
                    list_w,
                    fg,
                    bg,
                    &format!("▸{}", " ".repeat(usize::from(list_w.saturating_sub(1)))),
                )?;
            }
            f.text(2, y, name_w, fg, bg, &row.name)?;
            f.text(docker_x, y, 8, fg, bg, &row.status)?;
            f.text(map_x, y, list_w.saturating_sub(map_x), fg, bg, row.state())?;
        }
        if overlay.rows.is_empty() && status.is_empty() {
            f.label(
                1,
                3,
                list_w.saturating_sub(1),
                p.muted,
                "No saved mappings.",
            )?;
        }
    }
    if wide || details {
        let x = if wide { list_w + 2 } else { 1 };
        if wide {
            for y in 1..status_y {
                f.label(list_w, y, 1, p.rule, "│")?;
            }
        }
        f.label(x, 1, w.saturating_sub(x), p.accent, "Sandbox details")?;
        if let Some(row) = row {
            inspector(&mut f, p, row, overlay, x, status_y)?;
        } else {
            f.label(
                x,
                3,
                w.saturating_sub(x),
                p.muted,
                "Select a sandbox to inspect.",
            )?;
        }
    }
    if !status.is_empty() {
        f.label(1, status_y, w.saturating_sub(2), p.text, status)?;
    }
    for (i, line) in footer.iter().enumerate() {
        f.label(1, footer_y + i as u16, w.saturating_sub(2), p.accent, line)?;
    }
    f.flush()
}

fn interactive(overlay: &mut Overlay, rx: &Receiver<char>) -> u8 {
    let p = Palette::load();
    let mut selected: Option<(String, String)> = None;
    let mut first_frame = true;
    let mut message = String::new();
    let mut details = false;
    let mut notice_until = Instant::now();
    let _screen = if io::stdout().is_terminal() {
        match Screen::enter() {
            Ok(s) => Some(s),
            Err(_) => return 1,
        }
    } else {
        None
    };
    let mut frame = 0u64;
    while !overlay.stop.load(Ordering::Relaxed) {
        let listing = overlay.frame(frame % 3 == 0, io::stdout().is_terminal(), false);
        let rows = &overlay.rows;
        if first_frame {
            selected = rows.first().map(|r| (r.pane.clone(), r.name.clone()));
            first_frame = false;
        }
        let warnings = match listing {
            Err(e) => clean(&e),
            Ok(_) => overlay.warnings.join("  "),
        };
        let deadline = Instant::now() + REFRESH;
        loop {
            if overlay.stop.load(Ordering::Relaxed) {
                return 0;
            }
            let pos = rows.iter().position(|r| {
                selected
                    .as_ref()
                    .is_some_and(|(id, name)| id == &r.pane && name == &r.name)
            });
            let notice_active = !message.is_empty() && Instant::now() < notice_until;
            let status = if notice_active { &message } else { &warnings };
            if draw(overlay, pos, details, status, &p).is_err() {
                return 1;
            }
            let until = if notice_active {
                deadline.min(notice_until)
            } else {
                deadline
            };
            let Ok(key) = rx.recv_timeout(until.saturating_duration_since(Instant::now())) else {
                if Instant::now() < deadline {
                    continue;
                }
                break;
            };
            if key == 'i' {
                details = !details;
                continue;
            }

            if rows.is_empty() {
                break;
            }
            if matches!(key, 'j' | 'k') {
                let idx = match (pos, key) {
                    (Some(i), 'j') => (i + 1).min(rows.len() - 1),
                    (Some(i), _) => i.saturating_sub(1),
                    _ => 0,
                };
                selected = Some((rows[idx].pane.clone(), rows[idx].name.clone()));
                message.clear();
            } else if let Some((row, action)) =
                pos.and_then(|i| rows[i].action(key).map(|action| (&rows[i], action)))
            {
                message = match management::overlay_action(action, &row.snapshot) {
                    Ok(s) => {
                        if matches!(action, "forget-mapping" | "forget-missing") {
                            let idx = pos.unwrap();
                            selected = rows
                                .get(idx + 1)
                                .or_else(|| idx.checked_sub(1).and_then(|n| rows.get(n)))
                                .map(|r| (r.pane.clone(), r.name.clone()));
                        }
                        clean(&s)
                    }
                    Err(e) => clean(&e.message),
                };
                notice_until = Instant::now() + Duration::from_secs(4);
                while rx.try_recv().is_ok() {}
                break;
            }
        }
        frame += 1;
    }
    0
}

pub fn run(once: bool) -> u8 {
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let mut overlay = match Overlay::new(stop.clone()) {
        Ok(o) => o,
        Err(e) => {
            println!("{}", clean(&e));
            if !once {
                println!("Press any key to close (closes in 15s).");
                let _raw = input(stop.clone(), tx.clone(), true);
                wait(&stop, Duration::from_secs(15));
            }
            return 1;
        }
    };
    let _raw = if once {
        None
    } else {
        let sig = stop.clone();
        if let Err(e) = ctrlc::set_handler(move || sig.store(true, Ordering::Relaxed)) {
            eprintln!("{e}");
            return 1;
        }
        Some(input(stop.clone(), tx, false))
    };
    if !once {
        return interactive(&mut overlay, &rx);
    }
    match overlay.frame(true, true, true) {
        Ok(text) => {
            println!("{text}");
            0
        }
        Err(e) => {
            println!("could not read the sandboxes: {}", clean(&e));
            1
        }
    }
}
