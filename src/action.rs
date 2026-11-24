use crate::{config::Config, sbx::Failure};
use serde_json::{json, Value};
use std::{env, path::PathBuf};

fn directory(key: &str) -> Result<PathBuf, Failure> {
    env::var_os(key)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            Failure::new(
                "startup",
                format!("{key} is not set. Run this command through Herdr."),
            )
        })
}

pub fn context() -> Result<(PathBuf, PathBuf, Config, Value), Failure> {
    let state = directory("HERDR_PLUGIN_STATE_DIR")?;
    let config_dir = directory("HERDR_PLUGIN_CONFIG_DIR")?;
    let invocation = match env::var("HERDR_PLUGIN_CONTEXT_JSON") {
        Ok(raw) if !raw.is_empty() => serde_json::from_str(&raw).map_err(|e| {
            Failure::new(
                "startup",
                format!("HERDR_PLUGIN_CONTEXT_JSON is not valid JSON: {e}"),
            )
        })?,
        _ => Value::Null,
    };
    let config = Config::load(&config_dir).map_err(|e| Failure::new("config", e))?;
    Ok((state, config_dir, config, invocation))
}

pub fn emit(action: &str, result: Result<(Value, Vec<String>), Failure>) -> u8 {
    let (mut payload, lines, code) = match result {
        Ok((p, lines)) => (p, lines, 0),
        Err(e) => {
            let mut p = json!({"ok":false,"message":e.message,"errorKind":e.kind});
            eprintln!("{}", e.message);
            if !e.output.trim().is_empty() {
                p["output"] = json!(e.output.trim().chars().take(4000).collect::<String>());
                eprintln!("{}", e.output.trim());
            }
            (p, Vec::new(), 1)
        }
    };
    payload["schemaVersion"] = json!(1);
    payload["plugin"] = json!("sbx.sandbox");
    payload["action"] = json!(env::var("HERDR_PLUGIN_ACTION_ID").unwrap_or_else(|_| action.into()));
    println!("HERDR_SANDBOX_RESULT: {payload}");
    for line in lines {
        println!("{line}");
    }
    code
}
