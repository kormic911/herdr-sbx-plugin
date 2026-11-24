use crossterm::style::Color;
use std::{env, fs};

pub(crate) struct Palette {
    pub accent: Color,
    pub panel: Color,
    pub rule: Color,
    pub muted: Color,
    pub text: Color,
    pub error: Color,
    pub on_accent: Color,
}

fn rgb(n: u32) -> Color {
    Color::Rgb {
        r: (n >> 16) as u8,
        g: (n >> 8) as u8,
        b: n as u8,
    }
}

// These are the settings-overlay tokens from Herdr's app::state::Palette.
fn preset(name: &str) -> [Color; 7] {
    use Color::*;
    if name == "terminal" {
        return [DarkBlue, Reset, Reset, White, Reset, Red, DarkGrey];
    }
    let values = match name {
        "catppuccin-latte" | "latte" | "light" => [
            0x1e66f5, 0xeff1f5, 0xccd0da, 0x8c8fa1, 0x4c4f69, 0xd20f39, 0xe6e9ef,
        ],
        "tokyo-night" | "tokyonight" => [
            0x7aa2f7, 0x1a1b26, 0x24283b, 0x697196, 0xc0caf5, 0xf7768e, 0x1a1b26,
        ],
        "tokyo-night-day" | "tokyo-day" | "tokyonight-day" => [
            0x2e7de9, 0xe1e2e7, 0xc4c8da, 0x68709a, 0x3760bf, 0xf52a65, 0xd2d3da,
        ],
        "dracula" => [
            0xbd93f9, 0x282a36, 0x44475a, 0x828cb4, 0xf8f8f2, 0xff5555, 0x282a36,
        ],
        "nord" => [
            0x88c0d0, 0x2e3440, 0x3b4252, 0x646e82, 0xeceff4, 0xbf616a, 0x2e3440,
        ],
        "gruvbox" | "gruvbox-dark" => [
            0xd79921, 0x282828, 0x3c3836, 0xa89984, 0xebdbb2, 0xfb4934, 0x282828,
        ],
        "gruvbox-light" => [
            0x076678, 0xfbf1c7, 0xebdbb2, 0x7c6f64, 0x3c3836, 0x9d0006, 0xf2e5bc,
        ],
        "one-dark" | "onedark" => [
            0x61afef, 0x282c34, 0x2c313a, 0x737a87, 0xabb2bf, 0xe06c75, 0x282c34,
        ],
        "one-light" | "onelight" => [
            0x4078f2, 0xfafafa, 0xf0f0f1, 0x686b77, 0x383a42, 0xe45649, 0xf5f5f6,
        ],
        "solarized" | "solarized-dark" => [
            0x268bd2, 0x002b36, 0x073642, 0x657b83, 0x93a1a1, 0xdc322f, 0x002b36,
        ],
        "solarized-light" => [
            0x268bd2, 0xfdf6e3, 0xeee8d5, 0x586e75, 0x657b83, 0xdc322f, 0xeee8d5,
        ],
        "kanagawa" => [
            0x7e9cd8, 0x1f1f28, 0x2a2a37, 0x87867d, 0xdcd7ba, 0xc34043, 0x1f1f28,
        ],
        "kanagawa-lotus" | "lotus" => [
            0x4d699b, 0xf2ecbc, 0xdcd5ac, 0x8a8980, 0x545464, 0xc84053, 0xd5cea3,
        ],
        "rose-pine" | "rosepine" => [
            0xc4a7e7, 0x191724, 0x1f1d2e, 0x908caa, 0xe0def4, 0xeb6f92, 0x26233a,
        ],
        "rose-pine-dawn" | "rosepine-dawn" | "dawn" => [
            0x907aa9, 0xfaf4ed, 0xf2e9e1, 0x797593, 0x464261, 0xb4637a, 0xf2e9e1,
        ],
        "vesper" => [
            0xffc799, 0x1a1a1a, 0x232323, 0x7e7e7e, 0xffffff, 0xff8080, 0x101010,
        ],
        _ => [
            0x89b4fa, 0x181825, 0x313244, 0x7f849c, 0xcdd6f4, 0xf38ba8, 0x1e1e2e,
        ],
    };
    values.map(rgb)
}

fn color(value: &str) -> Option<Color> {
    use Color::*;
    let s = value.trim().to_ascii_lowercase();
    if let Some(hex) = s.strip_prefix('#') {
        let hex = match hex.len() {
            3 => hex.chars().flat_map(|c| [c, c]).collect(),
            6 => hex.to_owned(),
            _ => return None,
        };
        return u32::from_str_radix(&hex, 16).ok().map(rgb);
    }
    if let Some(body) = s.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
        let mut parts = body.split(',').map(|p| p.trim().parse::<u8>().ok());
        let c = Rgb {
            r: parts.next()??,
            g: parts.next()??,
            b: parts.next()??,
        };
        return parts.next().is_none().then_some(c);
    }
    Some(match s.as_str() {
        "reset" | "none" | "transparent" | "default" => Reset,
        "black" => Black,
        "white" => White,
        "gray" | "grey" => Grey,
        "darkgray" | "darkgrey" => DarkGrey,
        "red" => DarkRed,
        "green" => DarkGreen,
        "blue" => DarkBlue,
        "yellow" => DarkYellow,
        "cyan" => DarkCyan,
        "magenta" | "purple" => DarkMagenta,
        "lightred" => Red,
        "lightgreen" => Green,
        "lightyellow" => Yellow,
        "lightblue" => Blue,
        "lightmagenta" => Magenta,
        "lightcyan" => Cyan,
        _ => return None,
    })
}

impl Palette {
    pub fn load() -> Self {
        let doc = crate::bindings::config_path()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|s| s.trim_start_matches('\u{feff}').parse::<toml::Table>().ok());
        let theme = doc
            .as_ref()
            .and_then(|d| d.get("theme"))
            .and_then(|v| v.as_table());
        let light = env::var("COLORFGBG")
            .ok()
            .and_then(|s| s.rsplit(';').next()?.parse::<u8>().ok())
            .is_some_and(|bg| bg == 7 || (9..=15).contains(&bg));
        Self::from_theme(theme, light)
    }
    fn from_theme(theme: Option<&toml::Table>, light: bool) -> Self {
        let get = |key| theme.and_then(|t| t.get(key));
        let auto = get("auto_switch").and_then(|v| v.as_bool()) == Some(true);
        let (name_key, fallback) = match (auto, light) {
            (true, true) => ("light_name", "catppuccin-latte"),
            (true, false) => ("dark_name", "catppuccin"),
            _ => ("name", "catppuccin"),
        };
        let name = get(name_key)
            .and_then(|v| v.as_str())
            .unwrap_or(fallback)
            .to_ascii_lowercase()
            .replace([' ', '_'], "-");
        let mut colors = preset(&name);
        if let Some(custom) = get("custom") {
            let mode = if light { "light" } else { "dark" };
            for (i, key) in [
                "accent",
                "panel_bg",
                "surface0",
                "overlay1",
                "text",
                "red",
                "surface_dim",
            ]
            .iter()
            .enumerate()
            {
                let scoped = auto
                    .then(|| custom.get(mode))
                    .flatten()
                    .and_then(|m| m.get(key));
                if let Some(c) = scoped
                    .or_else(|| custom.get(key))
                    .and_then(|v| v.as_str())
                    .and_then(color)
                {
                    colors[i] = c;
                }
            }
        }
        let [accent, panel, rule, muted, text, error, dim] = colors;
        Self {
            accent,
            panel,
            rule,
            muted,
            text,
            error,
            on_accent: if panel == Color::Reset { dim } else { panel },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_use_the_herdr_preset_and_scoped_overrides() {
        let t: toml::Table = r##"
            name = "Tokyo Night"
            auto_switch = true
            light_name = "latte"
            dark_name = "nord"
            [custom]
            accent = "#abc"
            [custom.light]
            accent = "rgb(1,2,3)"
        "##
        .parse()
        .unwrap();
        let day = Palette::from_theme(Some(&t), true);
        assert_eq!(day.accent, rgb(0x010203));
        assert_eq!(day.panel, rgb(0xeff1f5));
        let night = Palette::from_theme(Some(&t), false);
        assert_eq!(night.accent, rgb(0xaabbcc));
        assert_eq!(night.panel, rgb(0x2e3440));
    }
}
