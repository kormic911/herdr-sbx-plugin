use crate::{
    action, inventory, list, ports,
    sbx::{self, Failure},
};
use serde_json::{json, Value};
use std::{
    env,
    path::Path,
    time::{Duration, SystemTime},
};
type ActionResult = Result<(Value, Vec<String>), Failure>;

pub fn field(ctx: &Value, key: &str, var: &str) -> Option<String> {
    ctx[key]
        .as_str()
        .map(str::to_owned)
        .or_else(|| env::var(var).ok())
}

fn herdr(args: &[&str]) -> Result<(), Failure> {
    let bin = env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into());
    let output =
        sbx::run_cancellable(&bin, args, Duration::from_secs(10), || false).map_err(|mut e| {
            if e.kind == "startup" {
                e.message = format!("Could not run the Herdr CLI ({bin}).");
            }
            e
        })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(Failure {
            kind: "unknown",
            message: format!("herdr {} failed ({}).", args.join(" "), output.status),
            output: output.combined,
        })
    }
}

pub fn open_overlay() -> ActionResult {
    action::context()?;
    let id = if cfg!(windows) {
        "sandboxes-windows"
    } else {
        "sandboxes"
    };
    open_pane(id)
}

pub fn open_pane(id: &str) -> ActionResult {
    let plugin = env::var("HERDR_PLUGIN_ID").unwrap_or_else(|_| "sbx.sandbox".into());
    herdr(&[
        "plugin",
        "pane",
        "open",
        "--plugin",
        &plugin,
        "--entrypoint",
        id,
        "--focus",
    ])?;
    Ok((json!({"ok":true,"entrypoint":id}), vec![]))
}

fn mappings(state: &Path) -> Result<Vec<(String, Value)>, Failure> {
    inventory::mappings(state).map_err(|e| Failure::new("startup", e))
}

pub fn focused(rows: &[(String, Value)], ctx: &Value) -> Result<Value, Failure> {
    let pane = field(ctx, "focused_pane_id", "HERDR_PANE_ID");
    if let Some((_, row)) = rows.iter().find(|(id, _)| Some(id) == pane.as_ref()) {
        return Ok(row.clone());
    }
    let workspace = field(ctx, "workspace_id", "HERDR_WORKSPACE_ID");
    let candidates: Vec<_> = rows
        .iter()
        .filter(|(id, row)| {
            workspace.as_ref().is_some_and(|w| {
                !w.is_empty()
                    && (row["workspaceId"] == w.as_str() || id.starts_with(&format!("{w}:")))
            })
        })
        .collect();
    let describe = |rows: &[&(String, Value)]| {
        rows.iter()
            .map(|(id, row)| format!("{id} ({})", row["sandboxName"].as_str().unwrap_or("?")))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match candidates.as_slice() {
        [(id, row)] => {
            if let Err(e) = herdr(&["pane", "get", id]) {
                if !regex::Regex::new(
                    r"(?i)pane_not_found|pane [^ ]+ not found|no such pane|unknown pane",
                )
                .unwrap()
                .is_match(&e.output)
                {
                    return Err(e);
                }
            }
            eprintln!("Using the workspace's only mapping, pane {id}.");
            return Ok(row.clone());
        }
        [] => {}
        _ => {
            return Err(Failure::new(
                "target",
                format!(
                    "This workspace has several sandbox mappings: {}. Focus the pane you mean.",
                    describe(&candidates)
                ),
            ))
        }
    }
    if !rows.is_empty() {
        let live = list::list_panes().map_err(|e| Failure::new("unknown", e))?;
        let missing: Vec<_> = rows.iter().filter(|(id, _)| !live.contains(id)).collect();
        match missing.as_slice() {
            [(id, row)] => {
                eprintln!("Using the sandbox mapped to missing pane {id}.");
                return Ok(row.clone());
            }
            [] => {}
            _ => {
                return Err(Failure::new(
                    "target",
                    format!(
                        "These panes are closed, and the plugin still has a sandbox mapping for each: {}.",
                        describe(&missing)
                    ),
                ))
            }
        }
    }
    Err(Failure::new("target","No sandbox mapping for this pane or workspace. Focus a mapped pane or run start-agent first."))
}

pub fn info() -> ActionResult {
    let (state, _, config, ctx) = action::context()?;
    let row = focused(&mappings(&state)?, &ctx)?;
    let kind = row["agentKind"].as_str().unwrap_or("");
    let (profile, argv) = config.agent(kind).map_err(|e| Failure::new("config", e))?;
    let name = row["sandboxName"].as_str().unwrap_or("");
    let live = list::list_sandboxes(&config.sbx);
    let sandbox = live
        .as_ref()
        .ok()
        .and_then(|m| m.get(name))
        .map(|status| json!({"name":name,"status":status}));
    let sandbox_error = live.err().map(|e|json!({"kind":e.kind,"message":e.message,"at":chrono::DateTime::<chrono::Utc>::from(SystemTime::now()).to_rfc3339_opts(chrono::SecondsFormat::Millis,true)}));
    let mut lines = vec![];
    let mut published = vec![];
    let mut ports_error = Value::Null;
    if sandbox.is_some() {
        match ports::query(name, |args| sbx::run(&config.sbx, args))
            .and_then(|v| inventory::ports(&v).map_err(|e| Failure::new("unknown", e)))
        {
            Ok(rows) => {
                for (host, guest) in rows {
                    let link = format!("sbx://{name}/{guest}");
                    let url = host.map(|h| format!("http://localhost:{h}"));
                    let rendered = ports::terminal_link(
                        name,
                        guest,
                        host,
                        url.as_ref().unwrap_or(&link),
                        cfg!(windows),
                    );
                    lines.push(format!("port {guest}: {rendered}"));
                    published
                        .push(json!({"hostPort":host,"sandboxPort":guest,"url":url,"link":link}));
                }
            }
            Err(e) => ports_error = json!({"kind":e.kind,"message":e.message}),
        }
    }
    let mut payload = json!({"mapping":row,"agent":{"kind":kind,"title":profile["title"],"sbxAgent":profile["sbxAgent"],"herdrDetectionKind":profile["herdrDetectionKind"],"launchArgv":argv},"sandbox":sandbox,"sandboxError":sandbox_error,"gitRemote":(row["workspaceMode"]=="clone").then(||format!("sandbox-{name}")),"ports":published,"portsError":ports_error});
    lines.insert(0, format!("{payload:#}"));
    payload["paneId"] = row["paneId"].clone();
    payload["ok"] = json!(true);
    Ok((payload, lines))
}

pub fn open_port() -> ActionResult {
    let (state, _, config, ctx) = action::context()?;
    let clicked = field(&ctx, "clicked_url", "HERDR_PLUGIN_CLICKED_URL");
    let (name, guest) = if let Some(link) = clicked.as_deref().filter(|s| !s.is_empty()) {
        let (name, guest) = ports::parse_link(link)
            .ok_or_else(|| Failure::new("target", "Unrecognized sandbox port link."))?;
        if !mappings(&state)?.iter().any(|(_, r)| {
            r["sandboxName"] == name
                || r["replacesSandboxNames"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|s| s == name))
        }) {
            return Err(Failure::new(
                "target",
                format!("Sandbox {name} is not managed by this plugin."),
            ));
        }
        (name.to_owned(), Some(guest))
    } else {
        let row = focused(&mappings(&state)?, &ctx)?;
        (row["sandboxName"].as_str().unwrap_or("").to_owned(), None)
    };
    let values = ports::query(&name, |args| sbx::run(&config.sbx, args))?;
    let port = inventory::published_ports(&values)
        .map_err(|e| Failure::new("unknown", e))?
        .into_iter()
        .find(|p| guest.is_none_or(|g| p.guest == g))
        .ok_or_else(|| {
            Failure::new(
                "target",
                match guest {
                    Some(g) => format!("Sandbox {name} does not publish port {g}."),
                    None => format!("Sandbox {name} publishes no ports."),
                },
            )
        })?;
    let host = port.host.ok_or_else(|| Failure {
        kind: "unknown",
        message: format!("No host port reported for {name}:{}.", port.guest),
        output: port.raw.to_string(),
    })?;
    let url = format!("http://localhost:{host}");
    let (opener, opened) = ports::open(&url);
    Ok((
        json!({"ok":true,"sandboxName":name,"sandboxPort":port.guest,"hostPort":host,"url":url,"opened":opened,"opener":opener}),
        vec![url],
    ))
}
