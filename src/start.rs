use crate::{
    action,
    config::Config,
    sbx::{self, Failure},
    state::{self, Mapping},
};
use serde_json::{json, Value};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

fn target(msg: &str) -> Failure {
    Failure::new("target", msg)
}
pub fn path_text(p: &Path) -> Result<&str, Failure> {
    p.to_str()
        .ok_or_else(|| target("The path must be valid UTF-8."))
}
fn canonical(p: &Path) -> Result<PathBuf, Failure> {
    if !p.is_absolute() || !p.is_dir() {
        return Err(target(
            "The workspace must be an existing absolute directory.",
        ));
    }
    ordinary_path(fs::canonicalize(p)?)
}

pub fn ordinary_path(path: PathBuf) -> Result<PathBuf, Failure> {
    #[cfg(windows)]
    {
        let s = path_text(&path)?;
        if let Some(unc) = s.strip_prefix(r"\\?\UNC\") {
            return Ok(PathBuf::from(format!(r"\\{unc}")));
        }
        if let Some(local) = s.strip_prefix(r"\\?\") {
            return Ok(PathBuf::from(local));
        }
    }
    Ok(path)
}
fn validate_root(root: &Path) -> Result<(), Failure> {
    if root.parent().is_none()
        || env::home_dir().and_then(|p| canonical(&p).ok()).as_deref() == Some(root)
    {
        return Err(target("Choose a project directory below the filesystem root and outside the home directory root."));
    }
    Ok(())
}
pub fn paths(ctx: &Value) -> Result<(PathBuf, PathBuf), Failure> {
    let cwd = ctx["focused_pane_cwd"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(Path::new);
    let selected = ctx
        .pointer("/worktree/checkout_path")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| ctx["workspace_cwd"].as_str().filter(|s| !s.is_empty()));
    let root = if let Some(path) = selected {
        canonical(Path::new(path))?
    } else {
        let cwd = cwd
            .ok_or_else(|| target("Focus a project workspace or pane before starting an agent."))?;
        let git = Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .ok()
            .filter(|o| o.status.success());
        match git {
            Some(o) => canonical(Path::new(String::from_utf8_lossy(&o.stdout).trim_end()))?,
            None => canonical(cwd)?,
        }
    };
    validate_root(&root)?;
    let workdir = cwd
        .and_then(|p| canonical(p).ok())
        .filter(|p| p.starts_with(&root))
        .unwrap_or_else(|| root.clone());
    Ok((root, workdir))
}
pub fn herdr(bin: &str, args: &[&str]) -> Result<Value, Failure> {
    let out =
        sbx::run_cancellable(bin, args, Duration::from_secs(10), || false).map_err(|mut e| {
            if e.kind == "startup" {
                e.message = format!("Could not run Herdr ({bin}).")
            }
            e
        })?;
    if !out.status.success() {
        return Err(Failure {
            kind: "unknown",
            message: format!("Herdr {} {} failed.", args[0], args[1]),
            output: out.combined,
        });
    }
    Ok(serde_json::from_str(&out.stdout).unwrap_or(Value::Null))
}

pub fn quote(s: &str, windows: bool) -> Result<String, Failure> {
    if s.chars().any(char::is_control) || (!windows && s.contains('\\')) {
        return Err(target(
            "A pane command argument cannot be safely entered in the configured shell.",
        ));
    }
    let inner = if windows {
        s.chars()
            .map(|c| {
                if matches!(c, '\'' | '\u{2018}'..='\u{201b}') {
                    format!("{c}{c}")
                } else {
                    c.to_string()
                }
            })
            .collect()
    } else {
        s.replace('\'', "'\\''")
    };
    Ok(format!("'{inner}'"))
}

pub fn pane_command(args: &[&str], detection: Option<&str>) -> Result<String, Failure> {
    let words = args
        .iter()
        .map(|s| quote(s, cfg!(windows)))
        .collect::<Result<Vec<_>, _>>()?
        .join(" ");
    if cfg!(windows) {
        return match detection{
            Some(kind)=>Ok(format!("& {{ $saved = $env:HERDR_AGENT; try {{ $env:HERDR_AGENT = {}; & {words} }} finally {{ $env:HERDR_AGENT = $saved }} }}",quote(kind,true)?)),
            None=>Ok(format!("& {words}"))
        };
    }
    match detection {
        Some(kind) => Ok(format!("env HERDR_AGENT={} {words}", quote(kind, false)?)),
        None => Ok(words),
    }
}

pub fn run() -> Result<(Value, Vec<String>), Failure> {
    let (state_dir, config_dir, config, ctx) = action::context()?;
    let (root, workdir) = paths(&ctx)?;
    let root = path_text(&root)?;
    let cwd = path_text(&workdir)?;
    let settings = &config.values;
    let kind = settings["agentKind"].as_str().unwrap();
    let (profile, _) = config.agent(kind).map_err(|e| Failure::new("config", e))?;
    checked(
        sbx::run(&config.sbx, &["version", "--json"])?,
        "checking sbx",
    )?;
    let suffix = state::nonce()?;
    let name = format!(
        "{}-{kind}-{suffix}",
        settings["sandboxNamePrefix"]
            .as_str()
            .unwrap()
            .to_ascii_lowercase()
    );
    let label = format!("sbx {kind} {}", &suffix[..6]);
    let bin = env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into());
    let source = ctx["focused_pane_id"]
        .as_str()
        .map(String::from)
        .or_else(|| env::var("HERDR_PANE_ID").ok());
    let workspace = ctx["workspace_id"]
        .as_str()
        .map(String::from)
        .or_else(|| env::var("HERDR_WORKSPACE_ID").ok());
    let ratio = settings["paneRatio"].to_string();
    let (args, pointer) = if settings["openIn"] == "tab" {
        let mut args = vec!["tab", "create"];
        if let Some(w) = workspace.as_deref() {
            args.extend(["--workspace", w]);
        }
        args.extend(["--label", &label, "--cwd", cwd, "--focus"]);
        (args, "/root_pane/pane_id")
    } else {
        let mut args = vec!["pane", "split"];
        args.extend(source.as_deref());
        args.extend([
            "--direction",
            settings["paneDirection"].as_str().unwrap(),
            "--ratio",
            &ratio,
            "--cwd",
            cwd,
            "--focus",
        ]);
        (args, "/pane/pane_id")
    };
    let reply = herdr(&bin, &args)?;
    let pane = reply
        .get("result")
        .unwrap_or(&reply)
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| Failure::new("unknown", "Herdr did not return the new pane ID."))?;
    let launch = state::nonce()?;
    let exe = ordinary_path(env::current_exe()?)?;
    let command = pane_command(
        &[
            path_text(&exe)?,
            "bridge-start",
            "--pane-id",
            pane,
            "--state-dir",
            path_text(&state_dir)?,
            "--config-dir",
            path_text(&config_dir)?,
            "--name",
            &name,
            "--launch-id",
            &launch,
            "--herdr-bin",
            &bin,
            "--sbx-bin",
            &config.sbx,
        ],
        profile["herdrDetectionKind"].as_str(),
    )?;
    let mapping = Mapping {
        dir: &state_dir,
        pane,
        name: &name,
    };
    mapping.change(|old| {
        if old.is_some() {
            return Err(Failure::new(
                "conflict",
                format!(
                    "Herdr reused mapped pane {pane}. Its existing sandbox mapping was preserved."
                ),
            ));
        }
        Ok(json!({
            "sandboxName":name, "workspaceId":workspace, "agentKind":kind,
            "sbxAgent":profile["sbxAgent"], "localPath":root, "workdir":cwd,
            "workspaceMode":settings["workspaceMode"], "lifecycleState":"provisional",
            "createdAt":null, "sourcePaneId":source, "replacesSandboxNames":[],
            "deletedSandboxNames":[], "lastError":null, "pendingLaunchId":launch
        }))
    })?;
    if let Err(e) = herdr(&bin, &["pane", "rename", pane, &label])
        .and_then(|_| herdr(&bin, &["pane", "run", pane, &command]))
    {
        // A CLI timeout can occur after delivery. Only mark an unclaimed launch.
        let _ = mapping.change(|old| {
            let mut row = old.ok_or_else(|| target("Mapping disappeared."))?;
            if row["pendingLaunchId"] != launch {
                return Err(target("Launch already claimed."));
            }
            row["pendingLaunchId"] = Value::Null;
            row["lifecycleState"] = json!("failed");
            row["lastError"] = error(&e);
            Ok(row)
        });
        return Err(e);
    }
    let _ = herdr(
        &bin,
        &[
            "notification",
            "show",
            "Docker Sandbox starting",
            "--body",
            &name,
            "--sound",
            "none",
        ],
    );
    Ok((
        json!({"ok":true,"paneId":pane,"sourcePaneId":source,"sandboxName":name,"agentKind":kind,"localPath":root,"workdir":cwd,"workspaceMode":settings["workspaceMode"],"openIn":settings["openIn"],"previousSandboxNames":[]}),
        vec![],
    ))
}

fn checked(out: sbx::Output, step: &str) -> Result<(), Failure> {
    if !out.status.success() {
        return Err(Failure {
            kind: sbx::classify(&out.combined),
            message: format!("sbx failed while {step} ({}).", out.status),
            output: out.combined,
        });
    }
    Ok(())
}
fn error(e: &Failure) -> Value {
    json!({"kind":e.kind,"message":e.message,"at":state::now()})
}
fn values<'a>(value: &'a Value, key: &str) -> impl Iterator<Item = &'a str> {
    value[key]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
}
fn create_args<'a>(s: &'a Value, row: &'a Value, name: &'a str, cpus: &'a str) -> Vec<&'a str> {
    let mut args = vec!["create", "--name", name];
    if let Some(t) = s["template"].as_str() {
        args.extend(["--template", t])
    }
    for (key, flag) in [
        ("kits", "--kit"),
        ("kitArgs", "--kit-arg"),
        ("env", "--env"),
        ("envFiles", "--env-file"),
        ("publish", "--publish"),
        ("denyNetwork", "--deny-network"),
    ] {
        for v in values(s, key) {
            args.extend([flag, v])
        }
    }
    if !s["cpus"].is_null() {
        args.extend(["--cpus", cpus])
    }
    if let Some(m) = s["memory"].as_str() {
        args.extend(["--memory", m])
    }
    if row["workspaceMode"] == "clone" {
        args.push("--clone")
    }
    args.extend([
        row["sbxAgent"].as_str().unwrap(),
        row["localPath"].as_str().unwrap(),
    ]);
    args.extend(values(s, "extraWorkspaces"));
    args
}
pub fn exec_args(
    row: &Value,
    s: &Value,
    argv: &[&str],
    interactive: bool,
    agent_env: bool,
) -> Result<Vec<String>, Failure> {
    let mut args = vec!["exec"];
    if interactive {
        args.extend(["--interactive", "--tty"])
    }
    let root = Path::new(
        row["localPath"]
            .as_str()
            .ok_or_else(|| target("Mapping has no local path."))?,
    );
    let cwd = row["workdir"].as_str().map(Path::new).unwrap_or(root);
    let mut relative = String::new();
    if row["workspaceMode"] != "clone" {
        if cfg!(windows) {
            relative = path_text(
                cwd.strip_prefix(root)
                    .map_err(|_| target("Workdir is outside the workspace."))?,
            )?
            .replace('\\', "/");
        } else {
            args.extend(["--workdir", path_text(cwd)?]);
        }
    }
    if agent_env {
        for item in values(s, "agentEnv") {
            args.extend(["--env", item])
        }
    }
    args.extend([row["sandboxName"].as_str().unwrap(), "--"]);
    if !relative.is_empty() {
        args.extend([
            "sh",
            "-c",
            r#"cd "./$1" && shift && exec "$@""#,
            "sh",
            &relative,
        ]);
    }
    args.extend(argv);
    Ok(args.into_iter().map(String::from).collect())
}
// Mappings store resolved paths. On Windows the same directory may arrive
// through its DOS short name or a verbatim path prefix.
fn recorded_directory(input: &str) -> Result<PathBuf, Failure> {
    let path = Path::new(input);
    let resolved = canonical(path)?;
    if resolved == ordinary_path(path.to_owned())? {
        return Ok(resolved);
    }
    #[cfg(windows)]
    {
        use std::{os::windows::fs::MetadataExt, path::Component};
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        // Accept alternate spellings only when no junction or symlink
        // redirected any component of the recorded path.
        if !path.components().any(|c| c == Component::ParentDir) {
            for ancestor in path.ancestors() {
                if fs::symlink_metadata(ancestor)?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT
                    != 0
                {
                    return Err(target(
                        "The recorded workspace path now crosses a reparse point.",
                    ));
                }
            }
            return Ok(resolved);
        }
    }
    Err(target("The recorded workspace path changed before launch."))
}

fn execute(
    mapping: &Mapping,
    mut entry: Value,
    config: &Config,
    bin: &str,
) -> Result<i32, Failure> {
    let settings = &config.values;
    let row = &entry;
    let kind = row["agentKind"]
        .as_str()
        .ok_or_else(|| target("Mapping has no agent kind."))?;
    let (Some(root), Some(cwd), Some(_agent)) = (
        row["localPath"].as_str(),
        row["workdir"].as_str().or(row["localPath"].as_str()),
        row["sbxAgent"].as_str(),
    ) else {
        return Err(target(
            "The mapping lacks its workspace, workdir or sandbox agent.",
        ));
    };
    let (profile, argv) = config.agent(kind).map_err(|e| Failure::new("config", e))?;
    let kind = kind.to_owned();
    let listed =
        || Ok::<_, Failure>(crate::list::list_sandboxes(&config.sbx)?.contains_key(mapping.name));
    let clone = row["workspaceMode"] == "clone";
    let existing_clone = clone && listed()?;
    if !existing_clone {
        let root = recorded_directory(root)?;
        let cwd = recorded_directory(cwd)?;
        validate_root(&root)?;
        if !cwd.starts_with(&root) {
            return Err(target("The recorded workdir is outside the workspace."));
        }
        entry["localPath"] = json!(root);
        entry["workdir"] = json!(cwd);
    }
    let existing = existing_clone || (!clone && listed()?);
    let row = &entry;
    let slow = Duration::from_millis(
        env::var("HERDR_SBX_SLOW_TIMEOUT_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|n| *n > 0)
            .unwrap_or(1_800_000),
    );
    if !existing {
        println!(
            "Creating Docker Sandbox {} for {}",
            mapping.name, row["localPath"]
        );
        let cpus = settings["cpus"]
            .as_f64()
            .map(|n| n.to_string())
            .unwrap_or_default();
        let create = create_args(settings, row, mapping.name, &cpus);
        if let Err(error) = checked(
            sbx::run_cancellable(&config.sbx, &create, slow, || false)?,
            "creating the sandbox",
        ) {
            if error.kind != "conflict" || !listed().unwrap_or(false) {
                return Err(error);
            }
        }
        mapping.patch(
            json!({"lifecycleState":"created","createdAt":state::now(),"setupScriptRanAt":null}),
        )?;
    }
    if !existing || row["setupScriptRanAt"].is_null() {
        if let Some(script) = profile["setupScript"].as_str().filter(|s| !s.is_empty()) {
            let args = exec_args(
                row,
                settings,
                &[settings["shell"].as_str().unwrap(), "-lc", script],
                false,
                false,
            )?;
            let args: Vec<_> = args.iter().map(String::as_str).collect();
            checked(
                sbx::run_cancellable(&config.sbx, &args, slow, || false)?,
                "running agent setup",
            )?;
            mapping.patch(json!({"setupScriptRanAt":state::now()}))?;
        }
    }
    let probe = exec_args(
        row,
        settings,
        &["sh", "-c", r#"command -v "$1""#, "sh", argv[0]],
        false,
        true,
    )?;
    let probe: Vec<_> = probe.iter().map(String::as_str).collect();
    let out = sbx::run(&config.sbx, &probe)?;
    if !out.status.success() && out.combined.trim().is_empty() {
        return Err(Failure::new(
            "config",
            format!(
                "Agent command {:?} is unavailable in {}.",
                argv[0], mapping.name
            ),
        ));
    }
    checked(out, "checking the agent command")?;
    let cmd = exec_args(row, settings, &argv, true, true)?;
    mapping
        .patch(json!({"lifecycleState":"ready","lastConnectedAt":state::now(),"lastError":null}))?;
    let report = settings["reportAgentStatus"] == true && profile["herdrDetectionKind"].is_null();
    if report {
        let _ = herdr(
            bin,
            &[
                "pane",
                "report-agent",
                mapping.pane,
                "--source",
                "sbx.sandbox",
                "--agent",
                &kind,
                "--state",
                "unknown",
            ],
        );
    }
    println!("Launching {kind} in {}...", mapping.name);
    let status = interactive(&config.sbx, cmd);
    if report {
        let _ = herdr(
            bin,
            &[
                "pane",
                "release-agent",
                mapping.pane,
                "--source",
                "sbx.sandbox",
                "--agent",
                &kind,
            ],
        );
    }
    let code = status?.code().unwrap_or(1);
    if code != 0
        && crate::list::list_sandboxes(&config.sbx)
            .is_ok_and(|live| !live.contains_key(mapping.name))
    {
        mapping.patch(json!({"lifecycleState":"missing","lastError":error(&Failure::new("not-found","The sandbox no longer exists."))}))?;
    }
    Ok(code)
}

pub fn interactive(bin: &str, cmd: Vec<String>) -> std::io::Result<std::process::ExitStatus> {
    let status = Command::new(bin).args(cmd).status();
    print!("\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1004l\x1b[?1006l\x1b[?2004l\x1b[?1049l\x1b[?25h\x1b[0m");
    status
}

pub fn bridge(args: &[String]) -> Result<i32, Failure> {
    let option = |flag: &str| {
        args.chunks_exact(2)
            .find(|p| p[0] == flag)
            .map(|p| p[1].as_str())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| Failure::new("startup", format!("Missing {flag}")))
    };
    let mapping = Mapping {
        dir: Path::new(option("--state-dir")?),
        pane: option("--pane-id")?,
        name: option("--name")?,
    };
    let launch = option("--launch-id")?;
    let sbx = option("--sbx-bin")?;
    let bin = option("--herdr-bin")?;
    let config_dir = option("--config-dir")?;
    let mappings =
        || crate::inventory::mappings(mapping.dir).map_err(|e| Failure::new("startup", e));
    let snapshot = mappings()?
        .into_iter()
        .find(|(id, _)| id == mapping.pane)
        .ok_or_else(|| target("Mapping is gone."))?
        .1;
    let locked = crate::management::names(&snapshot)?;
    let row = mapping.change_with_names(&locked, |old| {
        let mut row = old.ok_or_else(|| target("Mapping is gone."))?;
        if crate::management::names(&row)? != locked
            || row["sandboxName"] != mapping.name
            || row["pendingLaunchId"] != launch
            || (row["lifecycleState"] != "provisional" && row["launchMode"] != "resume")
            || !row["deletingPid"].is_null()
            || !row["bridgePid"].is_null()
        {
            return Err(Failure::new(
                "conflict",
                "This launch is stale or the mapping is already in use.",
            ));
        }
        for (id, other) in mappings()? {
            if id == mapping.pane {
                continue;
            }
            let shared = crate::management::names(&other)?;
            if shared.contains(&mapping.name)
                || (shared.iter().any(|n| locked.contains(n))
                    && (state::live_owner(&other["bridgePid"], &other["bridgeToken"])
                        || state::live_owner(&other["deletingPid"], &other["deletingToken"])))
            {
                return Err(Failure::new(
                    "conflict",
                    "Another mapping owns this sandbox.",
                ));
            }
        }
        row["pendingLaunchId"] = Value::Null;
        row["bridgePid"] = json!(std::process::id());
        row["bridgeToken"] = json!(state::own_token());
        row["bridgeStartedAt"] = json!(state::now());
        if let Some(checkpoints) = row["deletedSandboxNames"].as_array_mut() {
            if checkpoints.contains(&json!(mapping.name)) {
                checkpoints.retain(|name| name != mapping.name);
                row["setupScriptRanAt"] = Value::Null;
            }
        }
        row["bridgeLaunchId"] = json!(launch);
        if row["launchMode"] != "resume" {
            row["lifecycleState"] = json!("creating");
        }
        Ok(row)
    })?;
    let outcome = Config::load(Path::new(config_dir))
        .map_err(|e| Failure::new("config", e))
        .and_then(|mut config| {
            config.sbx = sbx.to_owned();
            execute(&mapping, row, &config, bin)
        });
    let mut patch = json!({"bridgePid":null,"bridgeToken":null,"bridgeStartedAt":null});
    if let Err(e) = &outcome {
        patch["lifecycleState"] = json!("failed");
        patch["lastError"] = error(e);
    }
    mapping.patch(patch)?;
    outcome
}
