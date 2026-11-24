mod support;

use serde_json::json;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use tempfile::tempdir;

fn command(root: &Path, cli: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_herdr-sbx"));
    cmd.arg("sandboxes")
        .env("HERDR_PLUGIN_CONFIG_DIR", root)
        .env("HERDR_PLUGIN_STATE_DIR", root)
        .env("HERDR_SBX_BIN", cli)
        .env("HERDR_BIN_PATH", cli);
    cmd
}

#[test]
fn shows_live_missing_and_unknown_without_writing_state() {
    let temp = tempdir().unwrap();
    let root = temp.path();
    let cli = support::compile_cli(root);
    let panes = root.join("panes");
    fs::create_dir(&panes).unwrap();
    for (id, name) in [("p1", "test-live"), ("p2", "test-gone")] {
        fs::write(panes.join(format!("{id}.json")),json!({"version":1,"paneId":id,"sandboxName":name,"agentKind":"codex","lifecycleState":"prepared","localPath":"/some dir"}).to_string()).unwrap();
    }
    let run = |invalid| {
        let mut cmd = command(root, &cli);
        if invalid {
            cmd.env("HERDR_TEST_INVENTORY", r#"[{"name":"test-live"},{}]"#);
        }
        let result = cmd.arg("--once").output().unwrap();
        assert!(result.status.success());
        String::from_utf8(result.stdout).unwrap()
    };
    let table = run(false);
    for expected in [
        "2 mappings",
        "p2 (closed)",
        "stopped",
        "MISSING",
        "8080->3000",
        if cfg!(windows) {
            "\x1b]8;;http://localhost:8080\x1b\\"
        } else {
            "\x1b]8;;sbx://test-live/3000\x1b\\"
        },
        "1 stale mapping",
        "/some dir",
    ] {
        assert!(table.contains(expected), "{table}");
    }
    let failed = run(true);
    assert!(failed.contains("sbx ls failed"));
    assert!(failed.contains("unknown"));
    assert!(!failed.contains("MISSING"));
    assert_eq!(fs::read_dir(&panes).unwrap().count(), 2);
}

#[test]
fn quit_interrupts_an_inflight_cli() {
    let temp = tempdir().unwrap();
    let root = temp.path();
    let cli = support::compile_cli(root);
    let flag = root.join("started");
    let mut child = command(root, &cli)
        .env("HERDR_TEST_HANG_FLAG", &flag)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let start = Instant::now();
    while !flag.exists() {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "CLI did not start"
        );
        thread::sleep(Duration::from_millis(10));
    }
    child.stdin.as_mut().unwrap().write_all(b"q").unwrap();
    while child.try_wait().unwrap().is_none() {
        if start.elapsed() > Duration::from_secs(6) {
            let _ = child.kill();
            panic!("q did not interrupt the CLI");
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(child.wait().unwrap().success());
}
