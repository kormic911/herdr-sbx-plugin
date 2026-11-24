mod support;
use serde_json::{json, Value};
use std::{
    env, fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

struct Fixture {
    root: TempDir,
    cli: std::path::PathBuf,
    workspace: std::path::PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let cli = support::compile_cli(root.path());
        let workspace = root.path().join("developer's project");
        fs::create_dir_all(workspace.join("sub")).unwrap();
        fs::write(root.path().join("config.json"),json!({"agentKind":"codex","publish":["3000"],"env":["BUILD=1"],"agentEnv":["RUN=1"],"agentArgs":{"codex":["--test"]}}).to_string()).unwrap();
        Self {
            root,
            cli,
            workspace,
        }
    }
    fn env(&self, c: &mut Command) {
        c.env("HERDR_TEST_START_DIR", self.root.path())
            .env("HERDR_PLUGIN_STATE_DIR", self.root.path())
            .env("HERDR_PLUGIN_CONFIG_DIR", self.root.path())
            .env("HERDR_SBX_BIN", &self.cli)
            .env("HERDR_BIN_PATH", &self.cli)
            .env("PATH", self.root.path());
        c.env_remove("HERDR_TEST_SBX_FAILURE");
    }
    fn start(&self, ctx: Value) -> Value {
        self.action("start-agent", ctx)
    }
    fn action(&self, id: &str, ctx: Value) -> Value {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_herdr-sbx"));
        self.env(&mut cmd);
        let out = cmd
            .arg(id)
            .env("HERDR_PLUGIN_ACTION_ID", id)
            .env("HERDR_PLUGIN_CONTEXT_JSON", ctx.to_string())
            .output()
            .unwrap();
        let stdout = String::from_utf8(out.stdout).unwrap();
        let payload: Value = serde_json::from_str(
            stdout
                .lines()
                .next()
                .unwrap()
                .strip_prefix("HERDR_SANDBOX_RESULT: ")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(payload["ok"], out.status.success());
        payload
    }
    fn bridge(&self, fail: bool) -> Output {
        let script = fs::read_to_string(self.root.path().join("command")).unwrap();
        let mut shell = if cfg!(windows) {
            let ps = Path::new(&env::var_os("SystemRoot").unwrap())
                .join("System32/WindowsPowerShell/v1.0/powershell.exe");
            let file = self.root.path().join("pane.ps1");
            fs::write(&file, format!("\u{feff}{script}\nexit $LASTEXITCODE")).unwrap();
            let mut c = Command::new(ps);
            c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
                .arg(file);
            c
        } else {
            let mut c = Command::new("/bin/sh");
            c.args(["-c", &script]);
            c
        };
        self.env(&mut shell);
        #[cfg(unix)]
        {
            // The user's shell provides env for the optional detection label.
            let bin = self.root.path().join("env");
            if !bin.exists() {
                std::os::unix::fs::symlink("/usr/bin/env", bin).unwrap();
            }
        }
        if fail {
            shell.env("HERDR_TEST_SBX_FAILURE", "create");
        }
        shell.output().unwrap()
    }
    fn mapping(&self) -> Value {
        let files: Vec<_> = fs::read_dir(self.root.path().join("panes"))
            .unwrap()
            .map(|p| p.unwrap().path())
            .collect();
        assert_eq!(files.len(), 1);
        serde_json::from_slice(&fs::read(&files[0]).unwrap()).unwrap()
    }
}

#[test]
fn native_start_and_interactive_bridge_with_no_node_or_git_bash_on_path() {
    let f = Fixture::new();
    let ctx = json!({"workspace_cwd":f.workspace,"focused_pane_cwd":f.workspace.join("sub"),"focused_pane_id":"w1:p1","workspace_id":"w1"});
    let start = f.start(ctx.clone());
    assert_eq!(start["ok"], true, "{start}");
    assert_eq!(start["sourcePaneId"], "w1:p1");
    assert_eq!(f.mapping()["lifecycleState"], "provisional");
    let out = f.bridge(false);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let row = f.mapping();
    assert_eq!(row["lifecycleState"], "ready");
    assert_eq!(row["sandboxName"], start["sandboxName"]);
    assert!(row["bridgePid"].is_null());
    let calls = fs::read_to_string(f.root.path().join("sbx-args")).unwrap();
    assert!(calls.contains("--publish\t3000"));
    assert!(calls.contains("--env\tBUILD=1"));
    assert!(calls.contains("exec\t--interactive\t--tty"));
    assert!(calls.contains("--env\tRUN=1"));
    assert!(calls.contains("codex\t--test"));
    assert!(
        !f.bridge(false).status.success(),
        "a replay must not create another sandbox"
    );
    assert_eq!(f.start(ctx)["errorKind"], "conflict");
    assert_eq!(f.mapping(), row);
    assert_eq!(
        fs::read_to_string(f.root.path().join("sbx-args")).unwrap(),
        calls
    );
}

#[test]
fn native_start_records_creation_failures_and_rejects_missing_workspace() {
    let f = Fixture::new();
    assert_eq!(f.start(json!({}))["errorKind"], "target");
    assert_eq!(f.start(json!({"workspace_cwd":f.workspace}))["ok"], true);
    assert!(!f.bridge(true).status.success());
    let row = f.mapping();
    assert_eq!(row["lifecycleState"], "failed");
    assert_eq!(row["lastError"]["kind"], "permission");
    assert!(row["bridgePid"].is_null());
}

#[test]
fn login_shell_registers_its_owner_and_preserves_the_mapping() {
    let f = Fixture::new();
    let ctx = json!({"workspace_cwd":f.workspace});
    assert_eq!(f.start(ctx)["ok"], true);
    assert!(f.bridge(false).status.success());
    let before = f.mapping();
    let pane = before["paneId"].as_str().unwrap();
    let opened = f.action("open-shell", json!({"focused_pane_id":pane}));
    assert_eq!(opened["mappedPaneId"], pane);
    assert_eq!(opened["sandboxName"], before["sandboxName"]);
    let out = f.bridge(false);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let attached: Value =
        serde_json::from_slice(&fs::read(f.root.path().join("shell-snapshot")).unwrap()).unwrap();
    assert_eq!(attached["shellPids"][0]["paneId"], pane);
    assert_eq!(attached["shellPids"][0]["shellPaneId"], opened["paneId"]);
    assert_eq!(f.mapping()["shellPids"], json!([]));
    assert_eq!(f.mapping()["lifecycleState"], before["lifecycleState"]);
    let calls = fs::read_to_string(f.root.path().join("sbx-args")).unwrap();
    let shell = calls.lines().last().unwrap();
    assert!(shell.contains("exec\t--interactive\t--tty"));
    assert!(!shell.contains("RUN=1"));
    assert!(shell.ends_with("bash\t-l"));

    // A deletion may own the same name through a predecessor on another pane.
    let other = json!({"version":1,"paneId":"other","sandboxName":"replacement","replacesSandboxNames":[before["sandboxName"]],"deletingPid":std::process::id()});
    fs::write(f.root.path().join("panes/other.json"), other.to_string()).unwrap();
    let out = f.bridge(false);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("deletion owns"));
    assert_eq!(
        fs::read_to_string(f.root.path().join("sbx-args")).unwrap(),
        calls
    );
}

#[test]
fn named_shell_resolves_two_orphans_and_splits_the_current_pane() {
    let f = Fixture::new();
    let panes = f.root.path().join("panes");
    fs::create_dir(&panes).unwrap();
    let write_row = |id: &str, name: &str| {
        fs::write(
            panes.join(format!("{id}.json")),
            json!({"version":1,"paneId":id,"sandboxName":name,"localPath":f.workspace}).to_string(),
        )
        .unwrap()
    };
    write_row("old-a", "sbx-first");
    write_row("old-b", "sbx-second");
    let invoke = |args: &[&str]| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_herdr-sbx"));
        f.env(&mut cmd);
        let out = cmd
            .arg("open-shell")
            .args(args)
            .env_remove("HERDR_PANE_ID")
            .env_remove("HERDR_WORKSPACE_ID")
            .env("HERDR_PLUGIN_CONTEXT_JSON", "{}")
            .output()
            .unwrap();
        let stdout = String::from_utf8(out.stdout).unwrap();
        serde_json::from_str::<Value>(
            stdout
                .lines()
                .next()
                .unwrap()
                .strip_prefix("HERDR_SANDBOX_RESULT: ")
                .unwrap(),
        )
        .unwrap()
    };
    assert_eq!(invoke(&[])["errorKind"], "target");
    assert_eq!(invoke(&["--sandbox", "sbx-unknown"])["errorKind"], "target");
    assert!(!f.root.path().join("split-args").exists());
    let chosen = invoke(&["--sandbox", "sbx-second"]);
    assert_eq!(chosen["ok"], true, "{chosen}");
    assert_eq!(chosen["mappedPaneId"], "old-b");
    assert_eq!(chosen["sandboxName"], "sbx-second");
    assert!(fs::read_to_string(f.root.path().join("split-args"))
        .unwrap()
        .starts_with("pane\tsplit\t--direction"));
    let failure = f.root.path().join("split-error");
    fs::write(&failure, "permission_denied").unwrap();
    assert_eq!(invoke(&["--sandbox", "sbx-second"])["ok"], false);
    assert!(!f.root.path().join("tab-args").exists());
    fs::write(&failure, "pane_not_found").unwrap();
    let reopened = invoke(&["--sandbox", "sbx-second"]);
    assert_eq!(reopened["ok"], true, "{reopened}");
    assert_eq!(reopened["mappedPaneId"], "old-b");
    assert_eq!(reopened["paneId"], "w1:p2");
    assert!(f.root.path().join("tab-args").is_file());
    write_row("old-c", "sbx-second");
    assert_eq!(invoke(&["--sandbox", "sbx-second"])["errorKind"], "target");
}
