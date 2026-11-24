mod support;
use serde_json::{json, Value};
use std::{fs, path::Path, process::Command};
use tempfile::tempdir;

fn doctor(dir: &Path, bin: &Path, extra: &[(&str, &str)]) -> (bool, Value, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_herdr-sbx"))
        .arg("doctor")
        .env("HERDR_PLUGIN_ACTION_ID", "doctor")
        .env("HERDR_PLUGIN_STATE_DIR", dir)
        .env("HERDR_PLUGIN_CONFIG_DIR", dir)
        .env("HERDR_SBX_BIN", bin)
        .env_remove("HERDR_PLUGIN_CONTEXT_JSON")
        .env_remove("HERDR_TEST_SBX_FAILURE")
        .envs(extra.iter().copied())
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let payload: Value = serde_json::from_str(
        stdout
            .lines()
            .next()
            .unwrap()
            .strip_prefix("HERDR_SANDBOX_RESULT: ")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(payload["plugin"], "sbx.sandbox");
    assert_eq!(payload["schemaVersion"], 1);
    assert_eq!(payload["action"], "doctor");
    assert_eq!(payload["ok"], output.status.success());
    (output.status.success(), payload, stdout)
}

#[test]
fn classifies_missing_binary_config_and_bad_context() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let missing = root.join("no sbx here");
    assert_eq!(doctor(root, &missing, &[]).1["errorKind"], "startup");
    for cfg in [
        json!({"unknownKey":true}),
        json!({"env":["1bad"]}),
        json!({"customAgents":[]}),
        json!({"agentArgs":{"codex":[""]}}),
    ] {
        fs::write(root.join("config.json"), cfg.to_string()).unwrap();
        assert_eq!(doctor(root, &missing, &[]).1["errorKind"], "config");
    }
    let result = doctor(root, &missing, &[("HERDR_PLUGIN_CONTEXT_JSON", "oops")]).1;
    assert_eq!(result["errorKind"], "startup");
    assert!(result["message"].as_str().unwrap().contains("CONTEXT_JSON"));
}

#[test]
fn reads_config_checks_daemon_and_returns_first_line_result() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let fake = support::compile_cli(root);
    let config = json!({"sbxBin":fake,"workspaceMode":"clone","template":"custom-kit","agentKind":"example",
        "customAgents":{"example":{"title":"Example","sbxAgent":"codex","command":["example"],"defaultArgs":["default"]}},
        "agentArgs":{"example":["custom arg"]}});
    fs::write(root.join("config.json"), config.to_string()).unwrap();
    let missing = root.join("unused");
    let (ok, payload, stdout) = doctor(root, &missing, &[]);
    assert!(ok, "{payload}");
    assert!(payload["node"].is_null());
    assert!(payload["executable"].is_string());
    assert!(payload["versionWarning"]
        .as_str()
        .unwrap()
        .contains("0.40.1 is older"));
    assert_eq!(payload["daemon"]["running"], true);
    assert_eq!(payload["template"], "custom-kit");
    assert_eq!(payload["workspaceMode"], "clone");
    assert!(stdout.contains("example (example custom arg)"));
    for (failure, kind, diagnostic) in [
        ("version", "authentication", "Not logged in"),
        ("daemon", "daemon", "offline"),
    ] {
        let (ok, result, _) = doctor(root, &missing, &[("HERDR_TEST_SBX_FAILURE", failure)]);
        assert!(!ok, "Expected {failure} to fail: {result}");
        assert_eq!(result["errorKind"], kind, "{result}");
        assert_eq!(result["output"], diagnostic, "{result}");
    }
}

#[cfg(unix)]
#[test]
fn kills_a_hung_cli_at_the_configured_deadline() {
    use std::{os::unix::fs::PermissionsExt, time::Instant};
    let tmp = tempdir().unwrap();
    let bin = tmp.path().join("hung");
    fs::write(&bin, "#!/bin/sh\nexec sleep 20\n").unwrap();
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
    let start = Instant::now();
    let (_, payload, _) = doctor(tmp.path(), &bin, &[("HERDR_SBX_TIMEOUT_MS", "30")]);
    assert_eq!(payload["errorKind"], "daemon");
    assert!(payload["message"].as_str().unwrap().contains("was killed"));
    assert!(start.elapsed().as_secs() < 5);
}
