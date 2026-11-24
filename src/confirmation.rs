use crate::{
    location::Location,
    screen::{wrap, Frame, Screen},
    theme::Palette,
};
use chrono::{DateTime, SecondsFormat, Utc};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    terminal,
};
use serde_json::{json, Value};
use std::{
    env,
    error::Error,
    fs,
    io::{self, IsTerminal, Write},
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, SystemTime},
};

fn now() -> DateTime<Utc> {
    SystemTime::now().into()
}

fn printable(text: &str) -> String {
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

fn field(label: &str, text: &str) {
    let text = printable(text);
    let width = 94 - label.len();
    let mut rest = text.as_str();
    let indent = " ".repeat(label.len());
    let mut prefix = label;
    while let Some((end, next)) = rest.char_indices().nth(width) {
        let head = &rest[..end];
        let cut = if next == ' ' {
            end
        } else {
            head.char_indices()
                .rev()
                .find_map(|(i, c)| match c {
                    ',' => Some(i + 1),
                    ' ' if i > 0 => Some(i),
                    _ => None,
                })
                .unwrap_or(end)
        };
        println!("  {prefix}{}", rest[..cut].trim_end());
        rest = rest[cut..].trim_start();
        prefix = &indent;
    }
    println!("  {prefix}{rest}");
}

enum Input {
    Answer(String),
    Closed,
    Interrupted,
}

const FIELDS: &[(&str, &str)] = &[
    ("action:   ", "action"),
    ("sandbox:  ", "sandboxName"),
    ("worktree: ", "localPath"),
    ("pane:     ", "paneId"),
    ("", "consequence"),
];

struct Raw;
impl Drop for Raw {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

fn prompt(
    request: &Value,
    remaining: impl Fn() -> Duration,
    interrupts: &mpsc::Receiver<Input>,
) -> io::Result<Input> {
    let bin = env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into());
    let snapshot = crate::sbx::run_cancellable(
        &bin,
        &["api", "snapshot"],
        remaining().min(Duration::from_secs(1)),
        || false,
    )
    .ok()
    .filter(|out| out.status.success())
    .and_then(|out| serde_json::from_str::<Value>(&out.stdout).ok());
    let session = snapshot
        .as_ref()
        .and_then(|s| s.pointer("/result/snapshot"))
        .unwrap_or(&Value::Null);
    let locations: Vec<_> = request["paneId"]
        .as_str()
        .unwrap()
        .split(", ")
        .map(|id| Location::resolve(session, id, None))
        .collect();
    let palette = Palette::load();
    terminal::enable_raw_mode()?;
    let _raw = Raw;
    let _screen = Screen::enter()?;
    let mut answer = String::new();
    let mut offset = 0usize;
    loop {
        let time = remaining();
        if time.is_zero() {
            return Ok(Input::Closed);
        }
        if interrupts.try_recv().is_ok() {
            return Ok(Input::Interrupted);
        }
        let (w, h) = terminal::size()?;
        let mut f = Frame::new(&palette, (w, h))?;
        let pad = if w >= 44 { 2 } else { 1 };
        let width = w.saturating_sub(2 * pad);
        let text = |key: &str| printable(request[key].as_str().unwrap());
        let mut lines = Vec::new();
        let names = text("sandboxName");
        let targets: Vec<_> = names.split(", ").collect();
        let group = if targets.len() == 1 {
            "Sandbox".to_owned()
        } else {
            format!("Sandboxes ({})", targets.len())
        };
        lines.push((palette.muted, group));
        for name in targets {
            lines.extend(
                wrap(name, width.into())
                    .into_iter()
                    .map(|v| (palette.accent, v)),
            );
        }
        lines.push((palette.text, String::new()));
        let mut fields = vec![("Worktree", text("localPath"))];
        for loc in &locations {
            fields.push(("Workspace", printable(&loc.workspace)));
            fields.push(("Pane", printable(&loc.pane)));
        }
        fields.push(("Action", text("action")));
        for (label, value) in fields {
            let parts = wrap(&value, usize::from(width.saturating_sub(11)));
            for (i, part) in parts.iter().enumerate() {
                let label = if i == 0 { label } else { "" };
                lines.push((palette.text, format!("{label:<11}{part}")));
            }
        }
        lines.push((palette.text, String::new()));
        lines.extend(
            wrap(&text("consequence"), width.into())
                .into_iter()
                .map(|v| (palette.error, v)),
        );
        let body_height = usize::from(h.saturating_sub(8));
        let max_offset = lines.len().saturating_sub(body_height);
        offset = offset.min(max_offset);
        let usable = h >= 10 && w >= 27;
        if usable {
            f.label(pad, 0, width, palette.error, "Delete sandbox?")?;
            let timer = format!("{}s left", time.as_secs_f32().ceil() as u64);
            f.label(
                w.saturating_sub(pad + timer.len() as u16),
                0,
                timer.len() as u16,
                palette.muted,
                &timer,
            )?;
            let rule = "─".repeat(width.into());
            f.label(pad, 1, width, palette.rule, &rule)?;
            for (i, (color, line)) in lines.iter().skip(offset).take(body_height).enumerate() {
                f.label(pad, 3 + i as u16, width, *color, line)?;
            }
            f.label(pad, h - 5, width, palette.rule, &rule)?;
            f.label(pad, h - 4, width, palette.text, "Type DELETE to confirm")?;
            let field = format!(" ▸ {answer}▏");
            f.text(
                pad,
                h - 3,
                width,
                palette.text,
                palette.rule,
                &" ".repeat(width.into()),
            )?;
            f.text(pad, h - 3, width, palette.text, palette.rule, &field)?;
            let footer = if max_offset > 0 && width < 39 {
                "↑↓ scroll  Esc cancel"
            } else if max_offset > 0 {
                "Enter submit   Esc cancel   ↑↓ scroll"
            } else {
                "Enter submit   Esc cancel"
            };
            f.label(pad, h - 1, width, palette.accent, footer)?;
        } else {
            f.label(0, 0, w, palette.error, "Enlarge popup to review deletion")?;
        }
        f.flush()?;
        if !event::poll(time.min(Duration::from_secs(1)))? {
            continue;
        }
        match event::read()? {
            Event::Key(k) if k.kind != KeyEventKind::Release => match k.code {
                KeyCode::Esc => return Ok(Input::Closed),
                KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(Input::Interrupted)
                }
                KeyCode::Up => offset = offset.saturating_sub(1),
                KeyCode::Down => offset = offset.saturating_add(1),
                KeyCode::PageDown => offset = offset.saturating_add(body_height),
                KeyCode::PageUp => offset = offset.saturating_sub(body_height),
                KeyCode::Backspace => {
                    answer.pop();
                }
                KeyCode::Enter if usable => return Ok(Input::Answer(answer)),
                KeyCode::Char(c) if usable && !c.is_control() => answer.push(c),
                _ => {}
            },
            Event::Paste(s) if usable => answer.push_str(&s),
            _ => {}
        }
    }
}

pub(crate) fn run() -> Result<u8, Box<dyn Error>> {
    let id = env::var("HERDR_SBX_CONFIRMATION_ID")?;
    if id.len() != 12
        || !id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("invalid confirmation request id".into());
    }
    let state = env::var_os("HERDR_PLUGIN_STATE_DIR")
        .filter(|s| !s.is_empty())
        .ok_or("missing state directory")?;
    let dir = PathBuf::from(state).join("confirmations");
    let request: Value =
        serde_json::from_slice(&fs::read(dir.join(format!("{id}.request.json")))?)?;
    if request["requestId"].as_str() != Some(&id) {
        return Err("confirmation request id mismatch".into());
    }
    let expiry = request["expiresAt"]
        .as_str()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok());
    let remaining = || {
        expiry
            .and_then(|date| (date.with_timezone(&Utc) - now()).to_std().ok())
            .unwrap_or(Duration::ZERO)
    };
    let (tx, rx) = mpsc::channel();
    let interrupt = tx.clone();
    ctrlc::set_handler(move || {
        let _ = interrupt.send(Input::Interrupted);
    })?;

    for &(_, key) in FIELDS {
        request[key]
            .as_str()
            .ok_or_else(|| format!("missing {key}"))?;
    }
    let answer = if io::stdin().is_terminal() && io::stdout().is_terminal() {
        prompt(&request, remaining, &rx)?
    } else {
        println!("Docker Sandbox deletion\n");
        for &(label, key) in FIELDS {
            field(
                label,
                request[key]
                    .as_str()
                    .ok_or_else(|| format!("missing {key}"))?,
            );
        }
        println!("  This prompt expires in {}s.", remaining().as_secs());
        print!("Type DELETE to confirm, anything else to cancel: ");
        io::stdout().flush()?;
        thread::spawn(move || {
            let mut line = String::new();
            let input = match io::stdin().read_line(&mut line) {
                Ok(n) if n > 0 => Input::Answer(line),
                _ => Input::Closed,
            };
            let _ = tx.send(input);
        });
        rx.recv_timeout(remaining()).unwrap_or(Input::Closed)
    };
    let confirmed =
        matches!(&answer, Input::Answer(text) if text.trim() == "DELETE" && !remaining().is_zero());
    let decision = if confirmed { "confirmed" } else { "cancelled" };
    let mut temp = tempfile::NamedTempFile::new_in(&dir)?;
    serde_json::to_writer(
        &mut temp,
        &json!({
            "requestId": id, "decision": decision, "decidedAt": now().to_rfc3339_opts(SecondsFormat::Millis, true)
        }),
    )?;
    temp.flush()?;
    temp.persist(dir.join(format!("{id}.decision.json")))?;
    println!(
        "{}",
        if confirmed {
            "Confirmed."
        } else {
            "Cancelled."
        }
    );
    Ok(if matches!(answer, Input::Interrupted) {
        130
    } else {
        0
    })
}
