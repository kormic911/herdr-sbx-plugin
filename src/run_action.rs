use crate::start;
use serde_json::Value;
use std::{
    env, thread,
    time::{Duration, Instant},
};

/// Invoke through Herdr so external callers use the same focus and plugin config as a keybinding.
pub fn run(args: &[String]) -> Result<u8, String> {
    let usage = "Usage: herdr-sbx run-action ACTION [seconds]";
    let (action, limit) = match args {
        [action] => (action, 90),
        [action, limit] => (action, limit.parse::<u64>().map_err(|_| usage)?),
        _ => return Err(usage.into()),
    };
    let bin = env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into());
    let call = |args: &[&str]| {
        start::herdr(&bin, args).map_err(|e| format!("{} {}", e.message, e.output.trim()))
    };
    let invocation = call(&[
        "plugin",
        "action",
        "invoke",
        action,
        "--plugin",
        "sbx.sandbox",
    ])?;
    let id = invocation
        .pointer("/result/log/log_id")
        .and_then(Value::as_str)
        .ok_or("Herdr returned no action log ID.")?;
    let start = Instant::now();
    let timeout = Duration::from_secs(limit);
    while start.elapsed() < timeout {
        let reply = call(&[
            "plugin",
            "log",
            "list",
            "--plugin",
            "sbx.sandbox",
            "--limit",
            "100",
        ])?;
        let logs = reply
            .pointer("/result/logs")
            .and_then(Value::as_array)
            .ok_or("Invalid Herdr log response.")?;
        if let Some(log) = logs
            .iter()
            .find(|log| log["log_id"] == id && log["status"] != "running")
        {
            if let Some(err) = log["stderr"].as_str().filter(|s| !s.trim().is_empty()) {
                eprintln!("{}", err.trim_end());
            }
            let line = log["stdout"]
                .as_str()
                .and_then(|s| s.lines().next())
                .unwrap_or("");
            println!("{line}");
            let ok = line
                .strip_prefix("HERDR_SANDBOX_RESULT: ")
                .and_then(|v| serde_json::from_str::<Value>(v).ok())
                .is_some_and(|v| v["ok"] == true);
            return Ok(if ok && log["status"] == "succeeded" {
                0
            } else {
                1
            });
        }
        thread::sleep(Duration::from_millis(250).min(timeout.saturating_sub(start.elapsed())));
    }
    Err(format!(
        "{action} did not finish within {limit} seconds (log {id})."
    ))
}
