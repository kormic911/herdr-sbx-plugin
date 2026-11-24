mod support;
use serde_json::{json, Value};
use std::{fs, path::PathBuf, process::Command};
use tempfile::TempDir;

struct Fixture {
    dir: TempDir,
    cli: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let cli = support::compile_cli(dir.path());
        fs::create_dir(dir.path().join("panes")).unwrap();
        Self { dir, cli }
    }
    fn mapping(&self, id: &str, name: &str) {
        let row = json!({"paneId":id,"version":1,"sandboxName":name,"workspaceId":"w1","agentKind":"codex","workspaceMode":"clone","replacesSandboxNames":["test-live"]});
        fs::write(
            self.dir.path().join("panes").join(format!("{id}.json")),
            row.to_string(),
        )
        .unwrap();
    }
    fn run(&self, action: &str, ctx: Value, env: &[(&str, &str)]) -> Value {
        let out = Command::new(env!("CARGO_BIN_EXE_herdr-sbx"))
            .arg(action)
            .env("HERDR_PLUGIN_STATE_DIR", self.dir.path())
            .env("HERDR_PLUGIN_CONFIG_DIR", self.dir.path())
            .env("HERDR_PLUGIN_CONTEXT_JSON", ctx.to_string())
            .env("HERDR_BIN_PATH", &self.cli)
            .env("HERDR_SBX_BIN", &self.cli)
            .env("HERDR_SBX_OPENER", &self.cli)
            .env_remove("HERDR_PLUGIN_ACTION_ID")
            .env_remove("HERDR_PANE_ID")
            .env_remove("HERDR_WORKSPACE_ID")
            .env_remove("HERDR_PLUGIN_CLICKED_URL")
            .envs(env.iter().copied())
            .output()
            .unwrap();
        let stdout = String::from_utf8(out.stdout).unwrap();
        let result: Value = serde_json::from_str(
            stdout
                .lines()
                .next()
                .unwrap()
                .strip_prefix("HERDR_SANDBOX_RESULT: ")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(result["ok"], out.status.success(), "{result}");
        result
    }
}

#[test]
fn reports_recorded_agent_clone_remote_and_published_ports() {
    let f = Fixture::new();
    f.mapping("p1", "test-live");
    fs::write(
        f.dir.path().join("config.json"),
        r#"{"agentKind":"gemini","agentArgs":{"codex":["--demo"]}}"#,
    )
    .unwrap();
    for ctx in [
        json!({"focused_pane_id":"p1"}),
        json!({"workspace_id":"w1"}),
    ] {
        let v = f.run("info", ctx, &[]);
        assert_eq!(v["sandbox"], json!({"name":"test-live","status":"stopped"}));
        assert_eq!(v["gitRemote"], "sandbox-test-live");
        assert_eq!(v["agent"]["launchArgv"], json!(["codex", "--demo"]));
        assert_eq!(
            v["ports"][0],
            json!({"hostPort":8080,"sandboxPort":3000,"url":"http://localhost:8080","link":"sbx://test-live/3000"})
        );
    }
    let v = f.run(
        "info",
        json!({"focused_pane_id":"p1"}),
        &[("HERDR_TEST_INVENTORY", "invalid")],
    );
    assert_eq!(v["ok"], true);
    assert_eq!(v["sandboxError"]["kind"], "unknown");
    assert_eq!(v["ports"], json!([]));
    let v = f.run(
        "info",
        json!({"focused_pane_id":"p1"}),
        &[("HERDR_TEST_PORTS", "bad")],
    );
    assert_eq!(v["portsError"]["kind"], "unknown");
}

#[test]
fn resolves_orphans_and_refuses_ambiguous_targets() {
    let f = Fixture::new();
    f.mapping("p1", "test-live");
    f.mapping("p2", "missing");
    assert_eq!(f.run("info", json!({}), &[])["paneId"], "p2");
    assert_eq!(
        f.run("info", json!({"workspace_id":"w1"}), &[])["errorKind"],
        "target"
    );
    f.mapping("p3", "another");
    assert_eq!(f.run("info", json!({}), &[])["errorKind"], "target");
}

#[test]
fn opens_only_published_ports_of_tracked_sandboxes() {
    let f = Fixture::new();
    f.mapping("p1", "test-live");
    let ctx = json!({"focused_pane_id":"p1"});
    let v = f.run(
        "open-port",
        ctx.clone(),
        &[("HERDR_TEST_LEGACY_PORTS", "1")],
    );
    assert_eq!(v["opened"], true, "{v}");
    assert_eq!(v["hostPort"], 8080);
    let v = f.run(
        "open-port",
        ctx.clone(),
        &[("HERDR_SBX_OPENER", "no-such-herdr-test-opener")],
    );
    assert_eq!(v["opened"], false);
    assert_eq!(v["ok"], true);
    let v = f.run(
        "open-port",
        ctx,
        &[("HERDR_TEST_PORTS", r#"[{"port":3000}]"#)],
    );
    assert_eq!(v["errorKind"], "unknown");
    assert_eq!(v["output"], r#"{"port":3000}"#);
    f.mapping("p1", "replacement");
    for url in [
        "sbx://test-live/0",
        "sbx://test-live/65536",
        "sbx://test-live/+3000",
        "sbx://test-live/3001",
        "sbx://--help/3000",
        "sbx://other/3000",
        "sbx://default/3000",
        "https://example.com/",
    ] {
        assert_eq!(
            f.run("open-port", json!({"clicked_url":url}), &[])["errorKind"],
            "target",
            "{url}"
        );
    }
    assert_eq!(
        f.run(
            "open-port",
            json!({"clicked_url":"sbx://test-live/3000"}),
            &[]
        )["opened"],
        true
    );
}

#[test]
fn overlay_action_opens_the_platform_pane_and_propagates_failures() {
    let f = Fixture::new();
    let v = f.run("open-overlay", json!({}), &[]);
    assert_eq!(v["action"], "sandboxes");
    assert_eq!(v["entrypoint"], "sandboxes");
    assert_eq!(
        f.run(
            "open-overlay",
            json!({}),
            &[("HERDR_BIN_PATH", "missing-herdr-test-cli")]
        )["errorKind"],
        "startup"
    );
}

#[test]
fn edit_config_opens_the_settings_popup() {
    let f = Fixture::new();
    let v = f.run("edit-config", json!({}), &[]);
    let pane = "config-editor";
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["entrypoint"], pane);
}
