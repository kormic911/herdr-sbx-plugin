use crate::{
    action, inventory,
    sbx::{self, Failure},
};
use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{json, Value};
use std::{
    env,
    time::{Duration, SystemTime},
};

pub fn list_sandboxes(bin: &str) -> Result<std::collections::BTreeMap<String, Value>, Failure> {
    let result = sbx::run(bin, &["ls", "--json"])?;
    if !result.status.success() {
        return Err(Failure {
            kind: sbx::classify(&result.combined),
            message: format!("sbx ls failed while listing sandboxes ({}).", result.status),
            output: result.combined,
        });
    }
    let value = serde_json::from_str(&result.stdout).map_err(|e| {
        Failure::new(
            "unknown",
            format!("sbx ls --json printed invalid JSON: {e}"),
        )
    })?;
    inventory::sandboxes(&value).map_err(|e| Failure::new("unknown", e))
}

pub fn list_panes() -> Result<std::collections::HashSet<String>, String> {
    let bin = env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into());
    let result = sbx::run_cancellable(&bin, &["pane", "list"], Duration::from_secs(10), || false)
        .map_err(|e| {
        if e.kind == "startup" {
            format!("Could not run {bin} pane list.")
        } else {
            format!("herdr pane list: {}", e.message)
        }
    })?;
    if !result.status.success() {
        return Err(format!(
            "herdr pane list failed ({}): {}",
            result.status,
            result.combined.trim()
        ));
    }
    let value = serde_json::from_str(&result.stdout)
        .map_err(|e| format!("herdr pane list printed invalid JSON: {e}"))?;
    inventory::pane_ids(&value)
}

pub fn run() -> Result<(Value, Vec<String>), Failure> {
    let (state, _, config, _) = action::context()?;
    let rows = inventory::mappings(&state).map_err(|e| Failure::new("startup", e))?;
    let live = list_sandboxes(&config.sbx);
    let panes = list_panes();
    let mut mappings = Vec::new();
    let mut lines = Vec::new();
    for (id, entry) in rows {
        let field = |k: &str| entry[k].as_str().unwrap_or("");
        let name = field("sandboxName");
        let exists = live.as_ref().ok().map(|map| map.contains_key(name));
        let status = live.as_ref().ok().and_then(|map| map.get(name));
        let pane = panes.as_ref().ok().map(|ids| ids.contains(&id));
        let suffix = match pane {
            Some(false) => " (pane gone)",
            None => " (pane ?)",
            _ => "",
        };
        let label = match exists {
            None => "unknown",
            Some(false) => "MISSING",
            Some(true) => status.and_then(Value::as_str).unwrap_or("exists"),
        };
        lines.push(format!(
            "{id}{suffix}\t{name}\t{}\t{}\t{label}\t{}",
            field("agentKind"),
            field("lifecycleState"),
            field("localPath")
        ));
        let mut row = json!({"paneId":id,"paneExists":pane,"paneError":panes.as_ref().err(),"exists":exists,"status":status});
        for key in [
            "sandboxName",
            "agentKind",
            "localPath",
            "workspaceMode",
            "lifecycleState",
        ] {
            if let Some(v) = entry.get(key) {
                row[key] = v.clone();
            }
        }
        if let Some(v) = entry
            .get("workdir")
            .filter(|v| !v.is_null())
            .or_else(|| entry.get("localPath"))
        {
            row["workdir"] = v.clone();
        }
        mappings.push(row);
    }
    if mappings.is_empty() {
        lines.push("No sandbox mappings.".into());
    }
    let sandbox_error = live.err().map(|error| {
        lines.push(format!("sbx ls failed ({}): {}", error.kind, error.message));
        let now =
            DateTime::<Utc>::from(SystemTime::now()).to_rfc3339_opts(SecondsFormat::Millis, true);
        json!({"kind":error.kind,"message":error.message,"at":now})
    });
    Ok((
        json!({"ok":true,"mappings":mappings,"sandboxError":sandbox_error}),
        lines,
    ))
}
