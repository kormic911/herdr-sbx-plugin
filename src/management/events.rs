use super::Host;
use crate::{config::Config, sbx::Failure, start};
use serde_json::Value;
use std::{
    env, fs, io,
    path::{Component, Path, PathBuf},
};

// The worktree can already be gone. Resolve symlinks in the surviving prefix
// and retain the complete missing suffix so sibling worktrees stay distinct.
fn worktree_path(path: &str) -> Result<PathBuf, Failure> {
    let mut absolute = PathBuf::new();
    for part in std::path::absolute(Path::new(path))?.components() {
        match part {
            Component::ParentDir => {
                absolute.pop();
            }
            Component::CurDir => {}
            _ => absolute.push(part.as_os_str()),
        }
    }
    for parent in absolute.ancestors() {
        match fs::canonicalize(parent) {
            Ok(prefix) => {
                let mut resolved = start::ordinary_path(prefix)?;
                resolved.extend(absolute.strip_prefix(parent).unwrap().components());
                return Ok(resolved);
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        }
    }
    start::ordinary_path(absolute)
}

pub fn run() -> Result<(), Failure> {
    if env::var("HERDR_PLUGIN_EVENT").as_deref() != Ok("worktree.removed") {
        println!("Ignoring unrelated event.");
        return Ok(());
    }
    let directory = |key| {
        env::var_os(key)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
    };
    let (Some(dir), Some(config_dir)) = (
        directory("HERDR_PLUGIN_STATE_DIR"),
        directory("HERDR_PLUGIN_CONFIG_DIR"),
    ) else {
        println!("No plugin state or config directory; nothing to clean up.");
        return Ok(());
    };
    let config = Config::load(&config_dir).map_err(|e| Failure::new("config", e))?;
    if config.values["cleanupOnWorktreeRemoved"] == false {
        println!("cleanupOnWorktreeRemoved is false; leaving sandboxes alone.");
        return Ok(());
    }
    let raw = env::var("HERDR_PLUGIN_EVENT_JSON").unwrap_or_default();
    let event: Value = if raw.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&raw).map_err(|e| {
            Failure::new(
                "startup",
                format!("HERDR_PLUGIN_EVENT_JSON is not valid JSON: {e}"),
            )
        })?
    };
    let Some(path) = event
        .pointer("/data/worktree/path")
        .filter(|v| !v.is_null())
        .or_else(|| event.pointer("/worktree/path"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    else {
        println!("Event carries no worktree path; nothing to clean up.");
        return Ok(());
    };
    let host = Host {
        dir,
        config_dir,
        config,
        ctx: Value::Null,
        bin: env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into()),
    };
    let removed_path = worktree_path(path)?;
    let mut matches = Vec::new();
    for (_, row) in host.rows()? {
        if let Some(local) = row["localPath"].as_str().filter(|s| !s.is_empty()) {
            if worktree_path(local)? == removed_path {
                matches.push(row);
            }
        }
    }
    if matches.is_empty() {
        println!("No sandbox mapped to {path}.");
        return Ok(());
    }
    if !host.confirm("worktree.removed", &matches)? {
        println!("Deletion was not confirmed; mappings kept.");
        return Ok(());
    }
    let mut deleted = Vec::new();
    let mut failures = 0;
    for row in matches {
        let pane = row["paneId"].as_str().unwrap_or("?");
        let name = row["sandboxName"].as_str().unwrap_or("?");
        match host.destroy(&row, None, false) {
            Ok(report) => {
                deleted.extend(
                    report["deleted"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(String::from),
                );
                println!("Forgot mapping for pane {pane} ({name}).");
            }
            Err(e) => {
                failures += 1;
                eprintln!(
                    "Could not clean up {name} for pane {pane}: {} {}",
                    e.message,
                    e.output.trim()
                );
            }
        }
    }
    if !deleted.is_empty() {
        let body = format!("{} (worktree {path} was removed)", deleted.join(", "));
        let _ = start::herdr(
            &host.bin,
            &[
                "notification",
                "show",
                "Docker Sandboxes removed",
                "--body",
                &body,
                "--sound",
                "none",
            ],
        );
    }
    if failures > 0 {
        return Err(Failure::new(
            "unknown",
            format!("Cleanup failed for {failures} mapping(s)."),
        ));
    }
    Ok(())
}
