pub mod events;
use crate::{
    action,
    config::Config,
    inventory, list, mapped, ports,
    sbx::{self, Failure},
    start,
    state::{self, Mapping},
};
use serde_json::{json, Value};
use std::{
    env, fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

pub type Report = Result<(Value, Vec<String>), Failure>;
fn conflict(message: &str) -> Failure {
    Failure::new("conflict", message)
}
fn field<'a>(row: &'a Value, key: &str) -> Result<&'a str, Failure> {
    row[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Failure::new("target", format!("Missing {key} in mapping.")))
}
pub fn names(row: &Value) -> Result<Vec<&str>, Failure> {
    let mut out = vec![field(row, "sandboxName")?];
    if !row["replacesSandboxNames"].is_null() {
        for item in row["replacesSandboxNames"]
            .as_array()
            .ok_or_else(|| conflict("Invalid predecessor list."))?
        {
            let name = item
                .as_str()
                .ok_or_else(|| conflict("Invalid predecessor name."))?;
            if !out.contains(&name) {
                out.push(name);
            }
        }
    }
    if out.iter().any(|n| !ports::valid_name(n)) {
        return Err(conflict("Invalid sandbox name."));
    }
    Ok(out)
}
fn targets(row: &Value) -> Result<Vec<&str>, Failure> {
    Ok(names(row)?
        .into_iter()
        .filter(|n| {
            !row["deletedSandboxNames"]
                .as_array()
                .is_some_and(|a| a.iter().any(|s| s == *n))
        })
        .collect())
}
pub fn checked(bin: &str, args: &[&str]) -> Result<String, Failure> {
    let out = sbx::run(bin, args)?;
    if out.status.success() {
        Ok(out.combined)
    } else {
        Err(Failure {
            kind: sbx::classify(&out.combined),
            message: format!("{bin} {} failed ({}).", args[0], out.status),
            output: out.combined,
        })
    }
}
fn timeout(key: &str, default: u64) -> Duration {
    Duration::from_millis(
        env::var(key)
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|n| *n > 0)
            .unwrap_or(default),
    )
}

struct Host {
    dir: PathBuf,
    config_dir: PathBuf,
    config: Config,
    ctx: Value,
    bin: String,
}
impl Host {
    fn rows(&self) -> Result<Vec<(String, Value)>, Failure> {
        inventory::mappings(&self.dir).map_err(|e| Failure::new("startup", e))
    }
    fn map<'a>(&'a self, row: &'a Value) -> Result<Mapping<'a>, Failure> {
        Ok(Mapping {
            dir: &self.dir,
            pane: field(row, "paneId")?,
            name: field(row, "sandboxName")?,
        })
    }
    fn pane_ids(&self) -> Result<std::collections::HashSet<String>, Failure> {
        list::list_panes().map_err(|e| Failure::new("unknown", e))
    }
    fn idle(&self, row: &Value, shells: bool) -> Result<(), Failure> {
        let involved = names(row)?;
        let panes = self.pane_ids()?;
        for (id, other) in self.rows()? {
            if !names(&other)?.iter().any(|n| involved.contains(n)) {
                continue;
            }
            if state::live_owner(&other["bridgePid"], &other["bridgeToken"])
                || state::live_owner(&other["deletingPid"], &other["deletingToken"])
                || (shells
                    && other["shellPids"].as_array().is_some_and(|a| {
                        a.iter().any(|s| state::live_owner(&s["pid"], &s["token"]))
                    }))
            {
                return Err(conflict(&format!(
                    "Pane {id} has a live agent, shell or deletion. Close the session and retry."
                )));
            }
            if panes.contains(&id) {
                let reply = start::herdr(&self.bin, &["pane", "get", &id])?;
                let pane = reply
                    .pointer("/result/pane")
                    .or_else(|| reply.get("pane"))
                    .filter(|p| p["pane_id"] == id)
                    .ok_or_else(|| conflict("Could not verify Herdr pane ownership."))?;
                if !pane["agent"].is_null() {
                    return Err(conflict(&format!("Exit the agent in pane {id} first.")));
                }
            }
        }
        Ok(())
    }
    fn confirm(&self, action: &str, rows: &[Value]) -> Result<bool, Failure> {
        let dir = self.dir.join("confirmations");
        fs::create_dir_all(&dir)?;
        let id = state::nonce()?;
        let request = dir.join(format!("{id}.request.json"));
        let decision = dir.join(format!("{id}.decision.json"));
        let ttl = timeout("HERDR_SBX_CONFIRMATION_TIMEOUT_MS", 60000);
        let now = chrono::DateTime::<chrono::Utc>::from(std::time::SystemTime::now());
        let end = now
            .checked_add_signed(
                chrono::TimeDelta::from_std(ttl)
                    .map_err(|_| conflict("Invalid confirmation timeout."))?,
            )
            .ok_or_else(|| conflict("Invalid deadline."))?;
        // Use each request's own deadline. Concurrent actions may use different timeouts.
        for file in fs::read_dir(&dir)?.flatten() {
            let name = file.file_name();
            let Some(id) = name.to_str().and_then(|n| n.strip_suffix(".request.json")) else {
                continue;
            };
            if id.len() != 12 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
                continue;
            }
            let expires = fs::read(file.path())
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .and_then(|v| chrono::DateTime::parse_from_rfc3339(v["expiresAt"].as_str()?).ok());
            if expires.is_some_and(|t| t < now) {
                let _ = fs::remove_file(dir.join(format!("{id}.decision.json")));
                let _ = fs::remove_file(file.path());
            }
        }
        let mut all = vec![];
        for row in rows {
            all.extend(targets(row)?);
        }
        let join = |k: &str| {
            rows.iter()
                .filter_map(|r| r[k].as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut consequence = "DELETE permanently removes the listed sandboxes.".to_string();
        if action == "worktree.removed" {
            consequence.insert_str(0, "Herdr removed this worktree. ");
        }
        if rows.iter().any(|r| r["workspaceMode"] == "clone") {
            consequence.push_str(" Clone mode: unfetched commits and sandbox remote refs may be lost. Keep a local branch first.");
        }
        let details = json!({"action":action,"requestId":id,"createdAt":now.to_rfc3339(),"expiresAt":end.to_rfc3339(),"sandboxName":all.join(", "),"paneId":join("paneId"),"localPath":join("localPath"),"consequence":consequence});
        let mut temp = tempfile::NamedTempFile::new_in(&dir)?;
        serde_json::to_writer(&mut temp, &details)
            .map_err(|e| Failure::new("startup", e.to_string()))?;
        temp.persist_noclobber(&request).map_err(|e| e.error)?;
        let result = (|| {
            let deadline = Instant::now() + ttl;
            let entry = if cfg!(windows) {
                "deletion-confirmation-windows"
            } else {
                "deletion-confirmation"
            };
            start::herdr(
                &self.bin,
                &[
                    "plugin",
                    "pane",
                    "open",
                    "--plugin",
                    "sbx.sandbox",
                    "--entrypoint",
                    entry,
                    "--env",
                    &format!("HERDR_SBX_CONFIRMATION_ID={id}"),
                    "--focus",
                ],
            )?;
            while Instant::now() < deadline {
                match fs::read(&decision) {
                    Ok(data) => {
                        let v: Value = serde_json::from_slice(&data)
                            .map_err(|_| conflict("Invalid confirmation decision."))?;
                        return Ok(v["requestId"] == id
                            && v["decision"] == "confirmed"
                            && v["decidedAt"]
                                .as_str()
                                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                                .is_some_and(|t| t >= now && t <= end));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        thread::sleep(Duration::from_millis(50))
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            Ok(false)
        })();
        let _ = fs::remove_file(decision);
        let _ = fs::remove_file(request);
        result
    }
    fn destroy(
        &self,
        snapshot: &Value,
        replacement: Option<Value>,
        orphan: bool,
    ) -> Result<Value, Failure> {
        let map = self.map(snapshot)?;
        let locked = names(snapshot)?;
        let expected = targets(snapshot)?;
        map.transaction(&locked, None, |old, file| {
            let mut row = old
                .filter(|r| r == snapshot)
                .ok_or_else(|| conflict("Mapping changed during confirmation. Retry."))?;
            self.idle(&row, true)?;
            let mut deleted = Vec::new();
            let mut missing = Vec::new();
            for name in expected {
                if orphan && self.pane_ids()?.contains(map.pane) {
                    return Err(conflict("The pane came back."));
                }
                self.idle(&row, true)?;
                match checked(&self.config.sbx, &["rm", "--force", name]) {
                    Ok(_) => deleted.push(name),
                    Err(e) if e.kind == "not-found" => {
                        if list::list_sandboxes(&self.config.sbx)?.contains_key(name) {
                            return Err(conflict(
                                "Removal reported not-found but the sandbox is still listed.",
                            ));
                        }
                        missing.push(name);
                    }
                    Err(e) => return Err(e),
                }
                let mut checkpoints = row["deletedSandboxNames"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                checkpoints.push(json!(name));
                row["deletedSandboxNames"] = Value::Array(checkpoints);
                if name == map.name {
                    row["lifecycleState"] = json!("missing");
                }
                row = state::save(file, map.pane, row)?;
            }
            if let Some(mut new) = replacement {
                new["deletedSandboxNames"] = row["deletedSandboxNames"].clone();
                new["replacesSandboxNames"] = json!(locked);
                state::save(file, map.pane, new)?;
            } else {
                fs::remove_file(file)?;
            }
            let report = json!({"ok": true, "paneId": map.pane, "sandboxName": map.name,
                                "deleted": deleted, "alreadyMissing": missing});
            Ok(report)
        })
    }
    fn forget_missing(&self, row: &Value) -> Result<bool, Failure> {
        let tracked = names(row)?;
        let map = self.map(row)?;
        map.transaction(&tracked, None, |current, file| {
            if current.as_ref() != Some(row) || self.pane_ids()?.contains(map.pane) {
                return Ok(false);
            }
            self.idle(row, true)?;
            if list::list_sandboxes(&self.config.sbx)?
                .keys()
                .any(|n| tracked.contains(&n.as_str()))
            {
                return Ok(false);
            }
            fs::remove_file(file)?;
            Ok(true)
        })
    }

    fn prune(&self) -> Report {
        let rows = self.rows()?;
        let live = list::list_sandboxes(&self.config.sbx)?;
        let panes = self.pane_ids()?;
        let (mut orphans, mut kept, mut deleted, mut pruned, mut failures) =
            (vec![], vec![], vec![], vec![], vec![]);
        for (id, row) in rows {
            let mut item = json!({"paneId":id,"sandboxName":row["sandboxName"]});
            let tracked = names(&row)?;
            let exists = tracked.iter().any(|n| live.contains_key(*n));
            if panes.contains(&id) {
                if !exists {
                    item["reason"] = json!("pane still open");
                    kept.push(item);
                }
                continue;
            }
            if exists {
                orphans.push(row);
                continue;
            }
            let removed = self.forget_missing(&row);
            match removed {
                Ok(true) => pruned.push(item),
                other => {
                    item["reason"] = json!(other
                        .err()
                        .map_or_else(|| "mapping changed".into(), |e| e.message));
                    kept.push(item);
                }
            }
        }
        let confirmed = if orphans.is_empty() {
            None
        } else {
            Some(self.confirm("prune-mappings", &orphans)?)
        };
        for row in orphans {
            let mut item = json!({"paneId":row["paneId"],"sandboxName":row["sandboxName"]});
            if confirmed == Some(true) {
                match self.destroy(&row, None, true) {
                    Ok(v) => {
                        deleted.extend(v["deleted"].as_array().into_iter().flatten().cloned());
                        deleted.extend(
                            v["alreadyMissing"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(Value::as_str)
                                .map(|n| json!(format!("{n} (already gone)"))),
                        );
                        continue;
                    }
                    Err(e) => {
                        item["reason"] = json!(e.message);
                        failures.push(json!({"paneId":row["paneId"],"sandboxName":row["sandboxName"],"message":e.message,"errorKind":e.kind}));
                    }
                }
            } else {
                item["reason"] = json!("deletion not confirmed");
            }
            kept.push(item);
        }
        Ok((
            json!({"ok":true,"pruned":pruned,"kept":kept,"deleted":deleted,"failures":failures,"orphansConfirmed":confirmed}),
            vec![format!(
                "Pruned {} mappings and deleted {} orphaned sandboxes.",
                pruned.len(),
                deleted.len()
            )],
        ))
    }
    fn new_pane(&self, row: &Value) -> Result<String, Failure> {
        let cwd = row["workdir"].as_str().unwrap_or(field(row, "localPath")?);
        let settings = &self.config.values;
        let ratio = settings["paneRatio"].to_string();
        let mut tab = vec!["tab", "create", "--focus"];
        let include_cwd = row["workspaceMode"] != "clone" || Path::new(cwd).is_dir();
        if include_cwd {
            tab.extend(["--cwd", cwd]);
        }
        let (reply, pointer) = if settings["openIn"] == "tab" {
            (start::herdr(&self.bin, &tab)?, "/root_pane/pane_id")
        } else {
            let anchor = mapped::field(&self.ctx, "focused_pane_id", "HERDR_PANE_ID");
            let mut args = vec!["pane", "split"];
            args.extend(anchor.as_deref());
            args.extend([
                "--direction",
                settings["paneDirection"].as_str().unwrap(),
                "--ratio",
                &ratio,
                "--focus",
            ]);
            if include_cwd {
                args.extend(["--cwd", cwd]);
            }
            match start::herdr(&self.bin, &args) {
                Ok(r) => (r, "/pane/pane_id"),
                Err(e)
                    if e.output
                        .lines()
                        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                        .any(|v| v.pointer("/error/code") == Some(&json!("pane_not_found"))) =>
                {
                    (start::herdr(&self.bin, &tab)?, "/root_pane/pane_id")
                }
                Err(e) => return Err(e),
            }
        };
        reply
            .get("result")
            .unwrap_or(&reply)
            .pointer(pointer)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .ok_or_else(|| conflict("Herdr did not return a pane ID."))
    }
    fn launch(&self, row: &Value, replacement: bool) -> Report {
        let map = self.map(row)?;
        let locked = names(row)?;
        let kind = field(row, "agentKind")?;
        let (profile, _) = self
            .config
            .agent(kind)
            .map_err(|e| Failure::new("config", e))?;
        let orphan = !self.pane_ids()?.contains(map.pane);
        let mut fresh = orphan;
        let mut snapshot = row.clone();
        loop {
            let pane = if fresh {
                self.new_pane(row)?
            } else {
                map.pane.to_owned()
            };
            let launch = state::nonce()?;
            let exe = start::ordinary_path(env::current_exe()?)?;
            let command = start::pane_command(
                &[
                    start::path_text(&exe)?,
                    "bridge-start",
                    "--pane-id",
                    &pane,
                    "--state-dir",
                    start::path_text(&self.dir)?,
                    "--config-dir",
                    start::path_text(&self.config_dir)?,
                    "--name",
                    map.name,
                    "--launch-id",
                    &launch,
                    "--herdr-bin",
                    &self.bin,
                    "--sbx-bin",
                    &self.config.sbx,
                ],
                profile["herdrDetectionKind"].as_str(),
            )?;
            let moved = map.pane != pane;
            map.transaction(&locked, Some(&pane), |current, file| {
                let mut current = current
                    .filter(|r| r == &snapshot)
                    .ok_or_else(|| conflict("The mapping changed before launch."))?;
                self.idle(&current, false)?;
                let launch_timeout = timeout("HERDR_SBX_BRIDGE_START_TIMEOUT_MS", 4000);
                let recent = current["updatedAt"]
                    .as_str()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .is_some_and(|t| {
                        std::time::SystemTime::from(t).elapsed().unwrap_or_default()
                            < launch_timeout
                    });
                if recent && !current["pendingLaunchId"].is_null() {
                    return Err(conflict("Another agent launch is pending."));
                }
                current["pendingLaunchId"] = json!(launch);
                current["launchMode"] = json!(if replacement { "create" } else { "resume" });
                current["bridgePid"] = Value::Null;
                current["deletingPid"] = Value::Null;
                let dest = state::mapping_path(&self.dir, &pane);
                if moved && dest.try_exists()? {
                    return Err(conflict("New pane already tracks a sandbox."));
                }
                state::save(&dest, &pane, current)?;
                if moved {
                    if let Err(e) = fs::remove_file(file) {
                        fs::remove_file(dest)?;
                        return Err(e.into());
                    }
                }
                Ok(())
            })?;
            let _ = start::herdr(
                &self.bin,
                &["pane", "rename", &pane, &format!("sbx {kind} {}", map.name)],
            );
            let sent = start::herdr(&self.bin, &["pane", "run", &pane, &command]);
            let deadline = Instant::now() + timeout("HERDR_SBX_BRIDGE_START_TIMEOUT_MS", 4000);
            let dest = Mapping {
                dir: &self.dir,
                pane: &pane,
                name: map.name,
            };
            loop {
                let current = self
                    .rows()?
                    .into_iter()
                    .find(|(id, _)| id == &pane)
                    .ok_or_else(|| conflict("Mapping disappeared during launch."))?
                    .1;
                if current["bridgeLaunchId"] == launch {
                    if current["lifecycleState"] == "failed" {
                        return Err(Failure::new(
                            "startup",
                            current["lastError"]["message"]
                                .as_str()
                                .unwrap_or("Sandbox launch failed."),
                        ));
                    }
                    return Ok((
                        json!({"ok":true,"paneId":pane,"sandboxName":map.name,"agentKind":kind,"mode":if moved || !matches!(row["lifecycleState"].as_str(),Some("ready"|"prepared"|"stopped")){"start"}else{"connect"},"adoptedFrom":orphan.then_some(map.pane),"movedTo":moved.then_some(&pane)}),
                        vec![],
                    ));
                }
                if sent.is_err() || Instant::now() >= deadline {
                    break;
                }
                thread::sleep(Duration::from_millis(40));
            }
            snapshot = dest.change(|r| {
                let mut r = r
                    .filter(|r| r["pendingLaunchId"] == launch)
                    .ok_or_else(|| conflict("Launch ownership changed."))?;
                r["pendingLaunchId"] = Value::Null;
                Ok(r)
            })?;
            sent?;
            if fresh {
                return Err(conflict(
                    "The bridge did not acknowledge the launch in the new pane.",
                ));
            }
            fresh = true;
        }
    }
    fn perform(&self, action: &str) -> Report {
        if action == "prune-mappings" {
            return self.prune();
        }
        let row = mapped::focused(&self.rows()?, &self.ctx)?;
        self.perform_row(action, row)
    }
    fn perform_row(&self, action: &str, row: Value) -> Report {
        let map = self.map(&row)?;
        let locked = names(&row)?;
        match action {
            "fetch-changes" => crate::fetch::run(&map, &row, &self.config.sbx),
            "reconnect" => {
                self.idle(&row, false)?;
                self.launch(&row, false)
            }
            "stop" => {
                map.transaction(&locked, None, |current, file| {
                    let mut r = current
                        .filter(|r| r == &row)
                        .ok_or_else(|| conflict("Mapping changed before stop."))?;
                    self.idle(&r, true)?;
                    match checked(&self.config.sbx, &["stop", map.name]) {
                        Ok(_) => {
                            if matches!(
                                r["lifecycleState"].as_str(),
                                Some("ready" | "prepared" | "stopped")
                            ) {
                                r["lifecycleState"] = json!("stopped");
                            }
                            r["lastError"] = Value::Null;
                        }
                        Err(e) => {
                            if e.kind == "not-found" {
                                r["lifecycleState"] = json!("missing");
                            }
                            r["lastError"] =
                                json!({"kind":e.kind,"message":e.message,"at":state::now()});
                            state::save(file, map.pane, r)?;
                            return Err(e);
                        }
                    }
                    state::save(file, map.pane, r)?;
                    Ok(())
                })?;
                let _ = start::herdr(
                    &self.bin,
                    &[
                        "notification",
                        "show",
                        "Docker Sandbox stopped",
                        "--body",
                        map.name,
                        "--sound",
                        "none",
                    ],
                );
                Ok((
                    json!({"ok":true,"paneId":map.pane,"sandboxName":map.name}),
                    vec![],
                ))
            }
            "forget-mapping" | "replace-sandbox" => {
                self.idle(&row, true)?;
                let replacement = if action == "replace-sandbox" {
                    let kind = self.config.values["agentKind"].as_str().unwrap();
                    let (profile, _) = self
                        .config
                        .agent(kind)
                        .map_err(|e| Failure::new("config", e))?;
                    let (root, cwd) = start::paths(
                        &json!({"workspace_cwd":field(&row,"localPath")?,"focused_pane_cwd":row["workdir"]}),
                    )?;
                    let name = format!(
                        "{}-{kind}-{}",
                        self.config.values["sandboxNamePrefix"]
                            .as_str()
                            .unwrap()
                            .to_ascii_lowercase(),
                        state::nonce()?
                    );
                    Some(
                        json!({"sandboxName":name,"agentKind":kind,"sbxAgent":profile["sbxAgent"],"workspaceMode":self.config.values["workspaceMode"],"workspaceId":row["workspaceId"],"sourcePaneId":row["sourcePaneId"],"lifecycleState":"provisional","localPath":root,"workdir":cwd,"createdAt":null,"lastError":null}),
                    )
                } else {
                    None
                };
                if !self.confirm(action, std::slice::from_ref(&row))? {
                    return Err(Failure::new("cancelled", "Deletion was not confirmed."));
                }
                let next = replacement.as_ref().map(|r| r["sandboxName"].clone());
                let mut report = self.destroy(&row, replacement, false)?;
                if let Some(name) = next {
                    let new = self
                        .rows()?
                        .into_iter()
                        .find(|(id, r)| id == map.pane && r["sandboxName"] == name)
                        .ok_or_else(|| conflict("Replacement mapping changed."))?
                        .1;
                    let (launch, _) = self.launch(&new, true)?;
                    report
                        .as_object_mut()
                        .unwrap()
                        .extend(launch.as_object().unwrap().clone());
                }
                Ok((report, vec![]))
            }
            _ => Err(Failure::new("target", "Unknown action.")),
        }
    }
}

impl Host {
    fn load() -> Result<Self, Failure> {
        let (dir, config_dir, config, ctx) = action::context()?;
        let bin = env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into());
        Ok(Self {
            dir,
            config_dir,
            config,
            ctx,
            bin,
        })
    }
}

pub fn overlay_action(action: &str, snapshot: &Value) -> Result<String, Failure> {
    let host = Host::load()?;
    let name = field(snapshot, "sandboxName")?;
    if action == "forget-missing" {
        return if host.forget_missing(snapshot)? {
            Ok(format!("Removed the stale mapping for {name}."))
        } else {
            Err(conflict(
                "The mapping or inventory changed. Refresh and try again.",
            ))
        };
    }
    let current = host.rows()?.into_iter().any(|(_, row)| &row == snapshot);
    if !current {
        return Err(conflict("Selected mapping changed. Refresh and try again."));
    }
    host.perform_row(action, snapshot.clone())?;
    Ok(match action {
        "stop" => format!("Stopped {name}."),
        "reconnect" => format!("Reconnected to {name}."),
        _ => format!("Removed the sandbox mapping for {name}."),
    })
}

pub fn run(action: &str) -> Report {
    if action == "install-keybindings" {
        let bin = env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into());
        return crate::bindings::run(&bin);
    }
    Host::load()?.perform(action)
}
