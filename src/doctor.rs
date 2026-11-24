use crate::sbx::{self, Failure};
use regex::Regex;
use serde_json::{json, Value};
use std::{env, path::PathBuf};

fn json_or_text(stdout: &str, fallback: &str) -> Value {
    serde_json::from_str(stdout)
        .ok()
        .filter(|v: &Value| !v.is_null())
        .unwrap_or_else(|| json!(fallback.trim()))
}

fn version_warning(version: &Value) -> Option<String> {
    let v = version
        .pointer("/client/version")
        .or_else(|| version.pointer("/server/version"))
        .unwrap_or(version);
    let pattern = Regex::new(r"([0-9]+)\.([0-9]+)\.([0-9]+)").unwrap();
    let text = v.to_string();
    let found = pattern.captures(&text)?;
    let parts: [u64; 3] = [
        found[1].parse().ok()?,
        found[2].parse().ok()?,
        found[3].parse().ok()?,
    ];
    (parts < [0,42,0]).then(||format!("sbx {}.{}.{} is older than 0.42.0; the plugin was written against the newer CLI and some flags may be missing.",parts[0],parts[1],parts[2]))
}

pub fn inspect() -> Result<(Value, Vec<String>), Failure> {
    let (state, config_dir, config, _) = crate::action::context()?;
    let root = match env::var_os("HERDR_PLUGIN_ROOT") {
        Some(p) => PathBuf::from(p),
        None => env::current_dir()?,
    };
    let version = sbx::run(&config.sbx, &["version", "--json"])?;
    if !version.status.success() {
        return Err(Failure {
            kind: sbx::classify(&version.combined),
            message: format!(
                "sbx version failed while reading the sbx version ({}).",
                version.status
            ),
            output: version.combined,
        });
    }
    let daemon = sbx::run(&config.sbx, &["daemon", "status", "--json"])?;
    if !daemon.status.success() {
        return Err(Failure {
            kind: "daemon",
            message: r#"The sbx CLI answers, but its daemon does not. Run "sbx daemon start" or "sbx diagnose"."#.into(),
            output: daemon.combined,
        });
    }
    let (_, launch) = config
        .agent(config.values["agentKind"].as_str().unwrap())
        .map_err(|s| Failure::new("config", s))?;
    let parsed = json_or_text(&version.stdout, &version.stdout);
    let warning = version_warning(&parsed);
    let agent = config.values["agentKind"].as_str().unwrap();
    let mode = config.values["workspaceMode"].as_str().unwrap();
    let client = parsed
        .pointer("/client/version")
        .or_else(|| parsed.pointer("/server/version"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| version.stdout.lines().next().unwrap_or(""));
    let exe = env::current_exe()?;
    let mut lines = vec![
        format!("sbx executable: {}", config.sbx),
        format!("Rust executable: {}", exe.display()),
        format!("plugin root: {}", root.display()),
        format!("state dir: {}", state.display()),
        format!("config dir: {}", config_dir.display()),
        format!("sbx version: {client}"),
        "daemon: reachable".into(),
        format!("configured agent: {agent} ({})", launch.join(" ")),
        format!("workspace mode: {mode}"),
    ];
    if let Some(s) = &warning {
        lines.push(format!("warning: {s}"));
    }
    Ok((
        json!({
            "ok": true,
            "sbxBin": config.sbx,
            "node": null,
            "executable": exe,
            "pluginRoot": root,
            "stateDir": state,
            "configDir": config_dir,
            "version": parsed,
            "versionWarning": warning,
            "daemon": json_or_text(&daemon.stdout, &daemon.combined),
            "agentKind": agent,
            "workspaceMode": mode,
            "template": config.values["template"]
        }),
        lines,
    ))
}
