use crate::theme::Palette;
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    execute, queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{
        Clear, ClearType, DisableLineWrap, EnableLineWrap, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};
use std::io::{self, Write};

pub(crate) struct Screen;
impl Screen {
    pub fn enter() -> io::Result<Self> {
        let guard = Self;
        execute!(io::stdout(), EnterAlternateScreen, DisableLineWrap, Hide)?;
        Ok(guard)
    }
}
impl Drop for Screen {
    fn drop(&mut self) {
        let _ = execute!(
            io::stdout(),
            ResetColor,
            Show,
            EnableLineWrap,
            LeaveAlternateScreen
        );
    }
}

// Reserve two cells for non-ASCII user text so wide path characters stay inside their column.
fn cells(c: char) -> usize {
    if c.is_ascii() || matches!(c, '↑' | '↓' | '←' | '→' | '─' | '│' | '▸' | '·' | '…')
    {
        1
    } else {
        2
    }
}

pub(crate) fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = vec![];
    let mut rest = text;
    let width = width.max(2);
    while !rest.is_empty() {
        let mut used = 0;
        let end = rest.char_indices().find_map(|(i, c)| {
            used += cells(c);
            (used > width).then_some(i)
        });
        let Some(end) = end else {
            lines.push(rest.to_owned());
            break;
        };
        let cut = rest[..end].rfind(' ').filter(|i| *i > 0).unwrap_or(end);
        lines.push(rest[..cut].to_owned());
        rest = rest[cut..].trim_start_matches(' ');
    }
    lines
}

pub(crate) struct Frame<'a> {
    bytes: Vec<u8>,
    pub w: u16,
    pub h: u16,
    palette: &'a Palette,
}
impl<'a> Frame<'a> {
    pub fn new(palette: &'a Palette, size: (u16, u16)) -> io::Result<Self> {
        let mut bytes = Vec::new();
        queue!(
            bytes,
            SetBackgroundColor(palette.panel),
            Clear(ClearType::All)
        )?;
        Ok(Self {
            bytes,
            w: size.0,
            h: size.1,
            palette,
        })
    }
    pub fn text(
        &mut self,
        x: u16,
        y: u16,
        width: u16,
        fg: Color,
        bg: Color,
        text: &str,
    ) -> io::Result<()> {
        if x >= self.w || y >= self.h {
            return Ok(());
        }
        let width = usize::from(width.min(self.w - x));
        let mut used = 0;
        let mut visible = String::new();
        for c in text.chars() {
            let c = if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') {
                '?'
            } else {
                c
            };
            used += cells(c);
            if used > width {
                break;
            }
            visible.push(c);
        }
        queue!(
            self.bytes,
            MoveTo(x, y),
            SetForegroundColor(fg),
            SetBackgroundColor(bg),
            Print(visible)
        )
    }
    pub fn label(&mut self, x: u16, y: u16, width: u16, fg: Color, text: &str) -> io::Result<()> {
        self.text(x, y, width, fg, self.palette.panel, text)
    }
    pub fn link(&mut self, x: u16, y: u16, link: &str) -> io::Result<()> {
        queue!(
            self.bytes,
            MoveTo(x, y),
            SetForegroundColor(self.palette.accent),
            SetBackgroundColor(self.palette.panel),
            Print(link)
        )
    }
    pub fn flush(&self) -> io::Result<()> {
        let mut out = io::stdout().lock();
        out.write_all(&self.bytes)?;
        out.flush()
    }
}
