mod support;

use serde_json::{json, Value};
use std::{fs, path::Path, process::Command};
use tempfile::tempdir;

fn run(root: &Path, sbx: &Path, herdr: &Path, extra: &[(&str, &str)]) -> (bool, Value, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_herdr-sbx"))
        .arg("list-sandboxes")
        .env("HERDR_PLUGIN_ACTION_ID", "list-sandboxes")
        .env("HERDR_PLUGIN_CONFIG_DIR", root)
        .env("HERDR_PLUGIN_STATE_DIR", root)
        .env("HERDR_SBX_BIN", sbx)
        .env("HERDR_BIN_PATH", herdr)
        .envs(extra.iter().copied())
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let result: Value = serde_json::from_str(
        stdout
            .lines()
            .next()
            .unwrap()
            .strip_prefix("HERDR_SANDBOX_RESULT: ")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(result["action"], "list-sandboxes");
    assert_eq!(result["ok"], output.status.success());
    (output.status.success(), result, stdout)
}

#[test]
fn reports_live_and_missing_and_supports_older_mappings() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let cli = support::compile_cli(root);
    assert_eq!(run(root, &cli, &cli, &[]).1["mappings"], json!([]));
    let dir = root.join("panes");
    fs::create_dir(&dir).unwrap();
    for (id, name) in [("p1", "test-live"), ("p2", "test-missing")] {
        fs::write(dir.join(format!("{id}.json")),json!({"version":1,"paneId":id,"sandboxName":name,"localPath":"/repo","agentKind":"codex","lifecycleState":"stopped","workspaceMode":"mount"}).to_string()).unwrap();
    }
    let (_, result, lines) = run(root, &cli, &cli, &[]);
    let rows = result["mappings"].as_array().unwrap();
    assert_eq!(
        rows[0],
        json!({"paneId":"p1","sandboxName":"test-live","exists":true,"paneExists":true,"status":"stopped","paneError":null,"localPath":"/repo","workdir":"/repo","workspaceMode":"mount","agentKind":"codex","lifecycleState":"stopped"})
    );
    assert_eq!(rows[1]["exists"], false);
    assert_eq!(rows[1]["paneExists"], false);
    assert!(rows[1]["status"].is_null());
    assert!(lines.contains("p2 (pane gone)"));
    let (_, no_status, _) = run(
        root,
        &cli,
        &cli,
        &[("HERDR_TEST_INVENTORY", r#"[{"name":"test-live"}]"#)],
    );
    assert_eq!(no_status["mappings"][0]["exists"], true);
    assert!(no_status["mappings"][0]["status"].is_null());
}

#[test]
fn incomplete_inventory_is_unknown_and_bad_state_fails() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let cli = support::compile_cli(root);
    let dir = root.join("panes");
    fs::create_dir(&dir).unwrap();
    let file = dir.join("mapping.json");
    fs::write(
        &file,
        r#"{"version":1,"paneId":"p1","sandboxName":"test-live"}"#,
    )
    .unwrap();
    let (ok, result, _) = run(
        root,
        &cli,
        &root.join("missing-herdr"),
        &[("HERDR_TEST_INVENTORY", r#"[{"name":"test-live"},{}]"#)],
    );
    assert!(ok);
    let row = &result["mappings"][0];
    assert!(row["exists"].is_null());
    assert!(row["paneExists"].is_null());
    assert!(row["paneError"].as_str().unwrap().contains("pane list"));
    assert_eq!(result["sandboxError"]["kind"], "unknown");
    assert!(result["sandboxError"]["at"].is_string());
    fs::write(&file, "broken").unwrap();
    assert_eq!(run(root, &cli, &cli, &[]).1["errorKind"], "startup");
}
