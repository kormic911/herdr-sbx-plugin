use crate::sbx::{self, Failure};
use serde_json::Value;
use std::{
    env, io,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub fn valid_name(name: &str) -> bool {
    name.len() >= 2
        && name != "default"
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-'))
}

pub fn terminal_link(
    name: &str,
    guest: u16,
    host: Option<u16>,
    label: &str,
    windows: bool,
) -> String {
    if !valid_name(name) {
        return label.to_owned();
    }
    let target = if windows {
        let Some(port) = host.filter(|p| *p > 0) else {
            return label.to_owned();
        };
        format!("http://localhost:{port}")
    } else {
        format!("sbx://{name}/{guest}")
    };
    format!("\x1b]8;;{target}\x1b\\{label}\x1b]8;;\x1b\\")
}

#[cfg(test)]
mod tests {
    #[test]
    fn windows_terminal_links_address_only_the_published_host_port() {
        assert_eq!(
            super::terminal_link("test-live", 3000, Some(8080), "8080->3000", true),
            "\x1b]8;;http://localhost:8080\x1b\\8080->3000\x1b]8;;\x1b\\"
        );
        assert_eq!(
            super::terminal_link("test-live", 3000, None, "?->3000", true),
            "?->3000"
        );
    }
}

pub fn parse_link(link: &str) -> Option<(&str, u16)> {
    let (name, port) = link.strip_prefix("sbx://")?.split_once('/')?;
    if !valid_name(name) || !port.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((name, port.parse().ok().filter(|n| *n > 0)?))
}

pub fn query(
    name: &str,
    mut run: impl FnMut(&[&str]) -> Result<sbx::Output, Failure>,
) -> Result<Value, Failure> {
    if !valid_name(name) {
        return Err(Failure::new(
            "config",
            format!("Invalid sandbox name {name:?}."),
        ));
    }
    let mut output = run(&["port", "ls", name, "--json"])?;
    // Older sbx releases expose the same inventory under the plural command.
    let diagnostic = output.combined.to_ascii_lowercase();
    if !output.status.success()
        && (diagnostic.contains("unknown command") || diagnostic.contains("unrecognized command"))
    {
        output = run(&["ports", name, "--json"])?;
    }
    if !output.status.success() {
        return Err(Failure {
            kind: sbx::classify(&output.combined),
            message: format!("Could not list ports for {name} ({}).", output.status),
            output: output.combined,
        });
    }
    serde_json::from_str(&output.stdout).map_err(|e| Failure {
        kind: "unknown",
        message: format!("Invalid sbx port inventory: {e}"),
        output: output.combined,
    })
}

pub fn open(url: &str) -> (String, bool) {
    let override_bin = env::var("HERDR_SBX_OPENER").ok().filter(|s| !s.is_empty());
    let default = if cfg!(windows) {
        "rundll32.exe"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let bin = override_bin.as_deref().unwrap_or(default);
    let mut command = Command::new(bin);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x00000008 | 0x00000200); // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
        if override_bin.is_none() {
            command.arg("url.dll,FileProtocolHandler");
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let result = (|| -> io::Result<()> {
        let mut child = command
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if let Some(status) = child.try_wait()? {
                return if status.success() {
                    Ok(())
                } else {
                    Err(io::Error::other(status.to_string()))
                };
            }
            thread::sleep(Duration::from_millis(25));
        }
        Ok(())
    })();
    if let Err(e) = &result {
        eprintln!("Could not open {url} with {bin}: {e}");
    }
    (bin.to_owned(), result.is_ok())
}
