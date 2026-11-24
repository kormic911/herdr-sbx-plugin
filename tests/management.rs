mod support;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::TempDir;
const PLUGIN: &str = env!("CARGO_BIN_EXE_herdr-sbx");
struct Fixture {
    dir: TempDir,
    herdr: PathBuf,
    sbx: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let herdr = support::compile_cli(dir.path());
        fs::create_dir(dir.path().join("panes")).unwrap();
        fs::write(dir.path().join("answer"), "DELETE").unwrap();
        Self {
            dir,
            sbx: herdr.clone(),
            herdr,
        }
    }
    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }
    fn rowfile(&self, id: &str) -> PathBuf {
        let hash = format!("{:x}", Sha256::digest(id.as_bytes()));
        self.path(&format!(
            "panes/{}-{}.json",
            id.replace(':', "_"),
            &hash[..10]
        ))
    }
    fn mapping(&self, id: &str, name: &str) -> Value {
        let r = json!({"version":1,"paneId":id,"sandboxName":name,"agentKind":"claude-code","sbxAgent":"claude","workspaceMode":"mount","localPath":self.dir.path(),"workdir":self.dir.path(),"lifecycleState":"ready"});
        fs::write(self.rowfile(id), r.to_string()).unwrap();
        r
    }
    fn configure(&self, cmd: &mut Command) {
        cmd.env("HERDR_PLUGIN_STATE_DIR", self.dir.path())
            .env("HERDR_PLUGIN_CONFIG_DIR", self.dir.path())
            .env("HERDR_TEST_START_DIR", self.dir.path())
            .env("HERDR_TEST_MANAGEMENT_DIR", self.dir.path())
            .env("HERDR_CONFIG_PATH", self.path("herdr.toml"))
            .env("HERDR_BIN_PATH", &self.herdr)
            .env("HERDR_SBX_BIN", &self.sbx)
            .env("HERDR_TEST_PLUGIN", PLUGIN);
    }

    // Stage the current Cargo test binary in an isolated plugin checkout so
    // the manifest launcher uses the binary under test.
    fn plugin_root(&self) -> PathBuf {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
        let root = self.path("linked plugin");
        let bin = root
            .join("target/release")
            .join(format!("herdr-sbx{}", std::env::consts::EXE_SUFFIX));
        if !bin.exists() {
            fs::create_dir_all(bin.parent().unwrap()).unwrap();
            fs::create_dir_all(root.join("bin")).unwrap();
            fs::copy(PLUGIN, &bin).unwrap();
            for script in [
                "events.ps1",
                "native-action.ps1",
                "resolve-sbx.ps1",
                "run-native.ps1",
            ] {
                fs::copy(repo.join("bin").join(script), root.join("bin").join(script)).unwrap();
            }
        }
        root
    }

    fn event(&self, name: &str, payload: &str) -> std::process::Output {
        let root = self.plugin_root();
        let manifest = include_str!("../herdr-plugin.toml")
            .parse::<toml::Table>()
            .unwrap();
        let platform = std::env::consts::OS;
        let event = manifest["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|table| {
                table["on"].as_str() == Some("worktree.removed")
                    && table["platforms"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|v| v.as_str() == Some(platform))
            })
            .unwrap();
        let mut argv = event["command"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap());
        let mut cmd = Command::new(argv.next().unwrap());
        self.configure(&mut cmd);
        cmd.args(argv)
            .current_dir(root)
            .env("HERDR_PLUGIN_EVENT", name)
            .env("HERDR_PLUGIN_EVENT_JSON", payload)
            .output()
            .unwrap()
    }

    fn action(&self, name: &str) -> Value {
        let root = self.plugin_root();
        let mut cmd = Command::new(root.join("target/release/herdr-sbx"));
        self.configure(&mut cmd);
        let out = cmd
            .current_dir(root)
            .arg("action")
            .env("HERDR_PLUGIN_ACTION_ID", name)
            .env("HERDR_PLUGIN_CONTEXT_JSON", r#"{"focused_pane_id":"p1"}"#)
            .output()
            .unwrap();
        let stdout = String::from_utf8(out.stdout).unwrap();
        let report: Value = serde_json::from_str(
            stdout
                .lines()
                .next()
                .unwrap()
                .strip_prefix("HERDR_SANDBOX_RESULT: ")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(report["ok"], out.status.success(), "{report}");
        report
    }
    fn ok(&self, name: &str) -> Value {
        let v = self.action(name);
        assert_eq!(v["ok"], true, "{v}");
        v
    }
}
#[test]
fn action_helper_returns_the_invoked_log_and_exit_status() {
    let f = Fixture::new();
    for ok in [true, false] {
        let marker = format!("HERDR_SANDBOX_RESULT: {}", json!({"ok":ok}));
        let logs = json!({"result":{"logs":[
            {"log_id":"another","status":"succeeded","stdout":"unrelated"},
            {"log_id":"selected","status":if ok {"succeeded"} else {"failed"},"stdout":format!("{marker}\nextra"),"stderr":"diagnostic"}
        ]}});
        fs::write(f.path("action-logs"), logs.to_string()).unwrap();
        let mut cmd = Command::new(PLUGIN);
        f.configure(&mut cmd);
        let out = cmd.args(["run-action", "info", "5"]).output().unwrap();
        assert_eq!(out.status.code(), Some(if ok { 0 } else { 1 }));
        assert_eq!(String::from_utf8(out.stdout).unwrap().trim_end(), marker);
        assert_eq!(
            String::from_utf8(out.stderr).unwrap().trim_end(),
            "diagnostic"
        );
    }
}

#[test]
fn retry_preserves_completed_setup_for_an_existing_sandbox() {
    let f = Fixture::new();
    let mut row = f.mapping("p1", "test-live");
    row["agentKind"] = json!("custom");
    row["lifecycleState"] = json!("failed");
    row["setupScriptRanAt"] = json!("2026-01-01T00:00:00Z");
    fs::write(f.rowfile("p1"), row.to_string()).unwrap();
    let config = json!({"customAgents":{"custom":{"title":"Test","sbxAgent":"claude","command":["claude"],"herdrDetectionKind":"claude","setupScript":"already-completed"}}});
    fs::write(f.path("config.json"), config.to_string()).unwrap();
    f.ok("reconnect");
    assert!(!fs::read_to_string(f.path("sbx-args"))
        .unwrap()
        .contains("already-completed"));
}

#[test]
fn stop_reconnect_replace_and_forget_through_native_bridge_and_popup() {
    let f = Fixture::new();
    f.mapping("p1", "test-live");
    f.ok("stop");
    f.ok("reconnect");
    assert!(!fs::read_to_string(f.path("sbx-args"))
        .unwrap()
        .starts_with("create"));
    let replacement = f.ok("replace-sandbox");
    assert_ne!(replacement["sandboxName"], "test-live");
    assert_eq!(replacement["deleted"], json!(["test-live"]));
    assert_eq!(
        f.ok("forget-mapping")["deleted"],
        json!([replacement["sandboxName"]])
    );
    assert!(!f.rowfile("p1").exists());
}
#[test]
fn deletion_cancels_checks_owners_and_checkpoints_partial_failures() {
    let f = Fixture::new();
    let mut row = f.mapping("p1", "test-live");
    row["replacesSandboxNames"] = json!(["older"]);
    fs::write(f.rowfile("p1"), row.to_string()).unwrap();
    fs::write(f.path("answer"), "CANCEL").unwrap();
    assert_eq!(f.action("forget-mapping")["errorKind"], "cancelled");
    let mut other = f.mapping("closed", "older");
    other["shellPids"] = json!([{"pid":std::process::id()}]);
    fs::write(f.rowfile("closed"), other.to_string()).unwrap();
    assert_eq!(f.action("forget-mapping")["errorKind"], "conflict");
    assert!(!f.path("removed").exists());
    fs::remove_file(f.rowfile("closed")).unwrap();
    fs::write(f.path("answer"), "DELETE").unwrap();
    fs::write(f.path("fail-rm"), "older").unwrap();
    assert_eq!(f.action("forget-mapping")["errorKind"], "permission");
    fs::remove_file(f.path("fail-rm")).unwrap();
    assert_eq!(f.ok("forget-mapping")["deleted"], json!(["older"]));
    assert_eq!(
        fs::read_to_string(f.path("removed"))
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["test-live", "older", "older"]
    );
}
#[test]
fn prune_drops_missing_and_confirms_live_orphans() {
    let f = Fixture::new();
    f.mapping("closed", "test-live");
    f.mapping("gone", "test-missing");
    f.mapping("p1", "still-open");
    let v = f.ok("prune-mappings");
    assert_eq!(v["deleted"], json!(["test-live"]));
    assert_eq!(v["pruned"][0]["paneId"], "gone");
    assert!(f.rowfile("p1").exists());
}
#[test]
fn keybindings_are_idempotent_and_restore_rejected_config() {
    let f = Fixture::new();
    let path = f.path("herdr.toml");
    let old = "# user config\n";
    fs::write(&path, old).unwrap();
    fs::write(f.path("reject-config"), "").unwrap();
    assert_eq!(f.action("install-keybindings")["ok"], false);
    assert_eq!(fs::read_to_string(&path).unwrap(), old);
    fs::remove_file(f.path("reject-config")).unwrap();
    assert_eq!(
        f.ok("install-keybindings")["added"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    let text = fs::read_to_string(&path).unwrap();
    for action in [
        "start-agent",
        "reconnect",
        "open-shell",
        "sandboxes",
        "edit-config",
    ] {
        assert!(text.contains(&format!("command = \"sbx.sandbox.{action}\"")));
    }
    assert_eq!(
        f.ok("install-keybindings")["existing"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    assert_eq!(fs::read_to_string(path).unwrap(), text);
}
fn git(dir: &Path, args: &[&str]) {
    assert!(Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .unwrap()
        .success());
}
#[test]
fn clone_fetch_imports_commits_and_suggests_branch() {
    let f = Fixture::new();
    let mut row = f.mapping("p1", "test-live");
    assert_eq!(f.action("fetch-changes")["errorKind"], "target");
    row["workspaceMode"] = json!("clone");
    fs::write(f.rowfile("p1"), row.to_string()).unwrap();
    assert_eq!(f.action("fetch-changes")["errorKind"], "not-found");
    let guest = f.path("guest");
    fs::create_dir(&guest).unwrap();
    git(&guest, &["init", "-b", "feature"]);
    git(
        &guest,
        &[
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "user.name=test",
            "commit",
            "--allow-empty",
            "-m",
            "fixture",
        ],
    );
    git(f.dir.path(), &["init"]);
    let bundle = f.path("guest.bundle");
    git(
        &guest,
        &["bundle", "create", bundle.to_str().unwrap(), "--branches"],
    );
    let v = f.ok("fetch-changes");
    assert_eq!(v["transport"], "bundle");
    assert_eq!(v["keep"][0]["local"], "feature");
    assert_eq!(v["branches"], json!(["sandbox-test-live/feature"]));
}
#[test]
#[ignore = "creates and deletes a real Docker Sandbox via the native confirmation flow"]
fn live_create_stop_delete() {
    live_cleanup(false);
}

#[test]
#[ignore = "creates a real Docker Sandbox and confirms worktree removal"]
fn live_create_worktree_cleanup() {
    live_cleanup(true);
}

fn live_cleanup(via_event: bool) {
    let mut f = Fixture::new();
    f.sbx = std::env::var_os("HERDR_SBX_BIN")
        .filter(|bin| !bin.is_empty())
        .unwrap_or_else(|| "sbx".into())
        .into();
    let mut bytes = [0; 6];
    getrandom::fill(&mut bytes).unwrap();
    let suffix = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let name = format!("herdr-test-{suffix}");
    let workspace = f.path("workspace");
    fs::create_dir(&workspace).unwrap();
    git(&workspace, &["init"]);
    eprintln!("Creating disposable sandbox {name} using {:?}", f.sbx);
    let created = Command::new(&f.sbx)
        .args(["create", "--name", &name, "claude"])
        .arg(&workspace)
        .status()
        .unwrap_or_else(|e| panic!("Could not launch {:?} to create {name}: {e}", f.sbx));
    assert!(created.success(), "sbx create failed for {name}");
    let mut row = f.mapping("p1", &name);
    row["localPath"] = json!(workspace);
    fs::write(f.rowfile("p1"), row.to_string()).unwrap();
    let stopped = f.action("stop");
    if via_event {
        let event = f.event(
            "worktree.removed",
            &json!({"data":{"worktree":{"path":workspace}}}).to_string(),
        );
        if !event.status.success() || f.rowfile("p1").exists() {
            let fallback = f.action("forget-mapping");
            panic!(
                "Worktree hook failed for {name}: {} {}. Cleanup retry: {fallback}",
                String::from_utf8_lossy(&event.stdout),
                String::from_utf8_lossy(&event.stderr)
            );
        }
    } else {
        let removed = f.action("forget-mapping");
        assert_eq!(
            removed["deleted"],
            json!([name]),
            "Cleanup failed for {name}: {removed}"
        );
    }
    let inventory = Command::new(&f.sbx)
        .args(["ls", "--json"])
        .output()
        .unwrap();
    assert!(inventory.status.success());
    let listing: Value = serde_json::from_slice(&inventory.stdout).unwrap();
    assert!(
        !listing.to_string().contains(&name),
        "Sandbox {name} survived cleanup."
    );
    assert_eq!(stopped["ok"], true, "{stopped}");
}

#[test]
fn changed_targets_and_inconsistent_inventories_preserve_mappings() {
    let f = Fixture::new();
    let mut row = f.mapping("p1", "test-live");
    row["replacesSandboxNames"] = json!(["added-during-popup"]);
    fs::write(f.path("changed-row"), row.to_string()).unwrap();
    assert_eq!(f.action("forget-mapping")["errorKind"], "conflict");
    assert!(!f.path("removed").exists());
    fs::remove_file(f.path("changed-row")).unwrap();
    fs::write(f.path("not-found"), "").unwrap();
    assert_eq!(f.action("forget-mapping")["errorKind"], "conflict");
    fs::write(f.path("panes-list"), r#"{"panes":[{}]}"#).unwrap();
    assert_eq!(f.action("prune-mappings")["ok"], false);
    assert!(f.rowfile("p1").exists());
}

#[test]
fn reconnect_adopts_closed_pane_and_reuses_its_sandbox() {
    let f = Fixture::new();
    f.mapping("closed", "test-live");
    let v = f.ok("reconnect");
    assert_eq!(v["adoptedFrom"], "closed");
    assert_eq!(v["paneId"], "w1:p2");
    assert!(f.rowfile("w1:p2").exists());
    assert!(!f.rowfile("closed").exists());
    assert!(!fs::read_to_string(f.path("sbx-args"))
        .unwrap()
        .contains("create\t"));
}

#[test]
fn reconnect_existing_clone_after_host_checkout_removal() {
    let f = Fixture::new();
    let mut row = f.mapping("closed", "test-live");
    row["workspaceMode"] = json!("clone");
    row["localPath"] = json!(f.path("removed"));
    row["workdir"] = row["localPath"].clone();
    fs::write(f.rowfile("closed"), row.to_string()).unwrap();
    f.ok("reconnect");
    assert!(!fs::read_to_string(f.path("split-args"))
        .unwrap()
        .contains("--cwd"));
    let args = fs::read_to_string(f.path("sbx-args")).unwrap();
    assert!(args.contains("exec"));
    assert!(!args.contains("create\t"));
}

#[test]
fn reconnect_refuses_missing_or_external_workdir_before_running_sbx() {
    let f = Fixture::new();
    let outside = tempfile::tempdir().unwrap();
    for workdir in [outside.path().to_owned(), f.path("removed-subdirectory")] {
        let mut row = f.mapping("p1", "test-live");
        row["workdir"] = json!(workdir);
        fs::write(f.rowfile("p1"), row.to_string()).unwrap();
        let result = f.action("reconnect");
        assert_eq!(result["ok"], false, "{result}");
        assert!(!f.path("sbx-args").exists());
    }
}

#[cfg(windows)]
#[test]
fn reconnect_accepts_verbatim_workspace_with_ordinary_nested_workdir() {
    let f = Fixture::new();
    let root = fs::canonicalize(f.dir.path()).unwrap();
    let workdir = f.path("nested project");
    fs::create_dir(&workdir).unwrap();
    let mut row = f.mapping("p1", "test-live");
    row["localPath"] = json!(root);
    row["workdir"] = json!(workdir);
    fs::write(f.rowfile("p1"), row.to_string()).unwrap();
    f.ok("reconnect");
    assert!(fs::read_to_string(f.path("sbx-args"))
        .unwrap()
        .contains("nested project"));
}

#[test]
fn worktree_event_confirms_all_matching_targets_and_keeps_other_worktrees() {
    let f = Fixture::new();
    let gone = f.path("removed project");
    for (id, name, path) in [
        ("p1", "one", gone.clone()),
        ("closed", "two", gone.clone()),
        ("other", "three", gone.join("nested")),
    ] {
        let mut row = f.mapping(id, name);
        row["localPath"] = json!(path);
        if id == "p1" {
            row["replacesSandboxNames"] = json!(["older", "already-gone"]);
            row["deletedSandboxNames"] = json!(["already-gone"]);
        }
        fs::write(f.rowfile(id), row.to_string()).unwrap();
    }
    let payload = json!({"data":{"worktree":{"path":gone}}}).to_string();
    fs::write(f.path("answer"), "CANCEL").unwrap();
    let cancel = f.event("worktree.removed", &payload);
    assert!(
        cancel.status.success(),
        "{}",
        String::from_utf8_lossy(&cancel.stderr)
    );
    assert!(!f.path("removed").exists());
    let popups: Vec<_> = fs::read_dir(f.dir.path())
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("popup-"))
        .collect();
    assert_eq!(popups.len(), 1);
    let popup: Value = serde_json::from_slice(&fs::read(popups[0].path()).unwrap()).unwrap();
    assert_eq!(popup["action"], "worktree.removed");
    let mut names: Vec<_> = popup["sandboxName"].as_str().unwrap().split(", ").collect();
    names.sort();
    assert_eq!(names, ["older", "one", "two"]);
    fs::write(f.path("answer"), "DELETE").unwrap();
    fs::write(f.path("fail-rm"), "two").unwrap();
    let partial = f.event("worktree.removed", &payload);
    assert!(!partial.status.success());
    assert!(!f.rowfile("p1").exists());
    assert!(f.rowfile("closed").exists());
    fs::remove_file(f.path("fail-rm")).unwrap();
    let retry = f.event(
        "worktree.removed",
        &json!({"worktree":{"path":gone}}).to_string(),
    );
    assert!(
        retry.status.success(),
        "{}",
        String::from_utf8_lossy(&retry.stderr)
    );
    assert!(!f.rowfile("closed").exists());
    assert!(f.rowfile("other").exists());
    let removed = fs::read_to_string(f.path("removed")).unwrap();
    let mut calls: Vec<_> = removed.lines().collect();
    calls.sort();
    assert_eq!(calls, ["older", "one", "two", "two"]);
}

#[test]
fn worktree_event_ignores_disabled_unrelated_and_incomplete_events() {
    let f = Fixture::new();
    f.mapping("p1", "keep-me");
    assert!(f.event("pane.closed", "{").status.success());
    assert!(f.event("worktree.removed", "{}").status.success());
    assert!(f
        .event(
            "worktree.removed",
            &json!({"worktree":{"path":f.path("other")}}).to_string()
        )
        .status
        .success());
    assert!(!f.event("worktree.removed", "{").status.success());
    fs::write(
        f.path("config.json"),
        r#"{"cleanupOnWorktreeRemoved":false}"#,
    )
    .unwrap();
    assert!(f
        .event(
            "worktree.removed",
            &json!({"worktree":{"path":f.dir.path()}}).to_string()
        )
        .status
        .success());
    assert!(f.rowfile("p1").exists());
    assert!(!f.path("confirmations").exists());
}

#[test]
fn overlay_cleans_only_the_selected_mapping_and_confirms_live_deletion() {
    use std::{
        io::{Read, Write},
        process::Stdio,
        sync::mpsc,
        thread,
        time::Duration,
    };
    let f = Fixture::new();
    f.mapping("p1", "test-live");
    f.mapping("p2", "test-gone");
    fs::write(f.path("panes-list"), r#"{"panes":[]}"#).unwrap();
    fs::write(f.path("answer"), "CANCEL").unwrap();

    let mut cmd = Command::new(PLUGIN);
    f.configure(&mut cmd);
    let mut ui = cmd
        .arg("sandboxes")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut screen = ui.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut bytes = [0; 4096];
        while let Ok(n) = screen.read(&mut bytes) {
            if n == 0 || tx.send(bytes[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let wait_for = |needle: &str| {
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        let mut output = Vec::new();
        while !String::from_utf8_lossy(&output).contains(needle) {
            output.extend(
                rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                    .expect(needle),
            );
        }
    };
    let mut keys = ui.stdin.take().unwrap();
    wait_for("r reconnect");
    keys.write_all(b"j").unwrap();
    wait_for("d remove stale mapping");
    keys.write_all(b"d").unwrap();
    wait_for("Removed the stale mapping for test-gone");
    assert!(f.rowfile("p1").exists());
    assert!(!f.rowfile("p2").exists());
    assert!(!f.path("removed").exists());

    keys.write_all(b"k").unwrap();
    wait_for("d delete");
    keys.write_all(b"d").unwrap();
    wait_for("Deletion was not confirmed");
    assert!(f.rowfile("p1").exists());
    assert!(!f.path("removed").exists());

    fs::write(f.path("answer"), "DELETE").unwrap();
    keys.write_all(b"d").unwrap();
    wait_for("Removed the sandbox mapping for test-live");
    keys.write_all(b"q").unwrap();
    assert!(ui.wait().unwrap().success());
    assert!(!f.rowfile("p1").exists());
    assert_eq!(
        fs::read_to_string(f.path("removed")).unwrap().trim(),
        "test-live"
    );
}
