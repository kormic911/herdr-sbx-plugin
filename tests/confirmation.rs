use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
};
use tempfile::tempdir;

const ID: &str = "abcdef012345";

fn popup(answer: &[u8], expiry: &str, names: &str) -> (String, Value) {
    let state = tempdir().unwrap();
    let dir = state.path().join("confirmations");
    fs::create_dir(&dir).unwrap();
    let request = json!({ "requestId":ID, "action":"forget-mapping", "paneId":"p1", "sandboxName":names,
        "localPath":"/w/\u{1b}[H", "consequence":"Delete all listed sandboxes.", "expiresAt":expiry });
    fs::write(dir.join(format!("{ID}.request.json")), request.to_string()).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_herdr-sbx"))
        .arg("deletion-confirmation")
        .env("HERDR_SBX_CONFIRMATION_ID", ID)
        .env("HERDR_PLUGIN_STATE_DIR", state.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(answer).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    let decision =
        serde_json::from_slice(&fs::read(dir.join(format!("{ID}.decision.json"))).unwrap())
            .unwrap();
    (text, decision)
}

#[test]
fn confirms_only_an_explicit_unexpired_request() {
    let future = "2099-01-01T00:00:00.000Z";
    for (input, expiry, expected) in [
        ("DELETE\n", future, "confirmed"),
        ("delete\n", future, "cancelled"),
        ("", future, "cancelled"),
        ("DELETE\n", "2000-01-01T00:00:00.000Z", "cancelled"),
        ("DELETE\n", "invalid", "cancelled"),
    ] {
        let (_, decision) = popup(input.as_bytes(), expiry, "example");
        assert_eq!(decision["decision"], expected);
        assert_eq!(decision["requestId"], ID);
        assert!(
            chrono::DateTime::parse_from_rfc3339(decision["decidedAt"].as_str().unwrap()).is_ok()
        );
    }
}

#[test]
fn every_target_is_displayed_without_terminal_controls() {
    let names: Vec<String> = (0..50).map(|n| format!("herdr-test-{n:012}")).collect();
    let (text, _) = popup(b"no\n", "2099-01-01T00:00:00Z", &names.join(", "));
    assert!(!text.contains(char::from(27)));
    assert!(text.contains("/w/?[H"));
    for name in names {
        assert!(text.contains(&name));
    }
}
