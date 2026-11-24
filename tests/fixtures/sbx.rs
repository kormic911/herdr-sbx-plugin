// Native child process for doctor integration tests on Windows and Unix.
use std::{env, process::ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if let Some(dir) = env::var_os("HERDR_TEST_MANAGEMENT_DIR") {
        use std::{
            fs,
            io::Write,
            path::PathBuf,
            process::{Command, Stdio},
        };
        let dir = PathBuf::from(dir);
        let file = |name: &str| dir.join(name);
        let is = |prefix: &[&str]| {
            args.iter()
                .map(String::as_str)
                .take(prefix.len())
                .eq(prefix.iter().copied())
        };
        if is(&["plugin", "action", "invoke"]) {
            println!(r#"{{"result":{{"log":{{"log_id":"selected"}}}}}}"#);
            return ExitCode::SUCCESS;
        }
        if is(&["plugin", "log", "list"]) {
            println!("{}", fs::read_to_string(file("action-logs")).unwrap());
            return ExitCode::SUCCESS;
        }
        if is(&["plugin", "pane", "open"]) {
            let id = args
                .iter()
                .find_map(|a| a.strip_prefix("HERDR_SBX_CONFIRMATION_ID="))
                .unwrap();
            fs::copy(
                file(&format!("confirmations/{id}.request.json")),
                file(&format!("popup-{id}")),
            )
            .unwrap();
            let answer = fs::read_to_string(file("answer")).unwrap();
            let mut child = Command::new(env::var_os("HERDR_TEST_PLUGIN").unwrap())
                .arg("deletion-confirmation")
                .env("HERDR_SBX_CONFIRMATION_ID", id)
                .stdin(Stdio::piped())
                .spawn()
                .unwrap();
            writeln!(child.stdin.take().unwrap(), "{answer}").unwrap();
            assert!(child.wait().unwrap().success());
            if file("changed-row").exists() {
                let row = fs::read_dir(file("panes"))
                    .unwrap()
                    .map(|e| e.unwrap().path())
                    .find(|p| p.extension().is_some_and(|e| e == "json"))
                    .unwrap();
                fs::copy(file("changed-row"), row).unwrap();
            }
            return ExitCode::SUCCESS;
        }
        if is(&["cp"]) {
            fs::copy(file("guest.bundle"), &args[2]).unwrap();
            return ExitCode::SUCCESS;
        }
        if is(&["pane", "run"]) {
            let mut child = if cfg!(windows) {
                let mut ps = Command::new("powershell.exe");
                ps.args(["-NoProfile", "-NonInteractive", "-Command", &args[3]]);
                ps
            } else {
                let mut sh = Command::new("/bin/sh");
                sh.args(["-c", &args[3]]);
                sh
            };
            return ExitCode::from(child.status().unwrap().code().unwrap_or(1) as u8);
        }
        if is(&["api","snapshot"]) && file("panes-list").exists() {
            // The fixture file contains a top-level panes array, like a session snapshot.
            println!("{{\"result\":{{\"snapshot\":{}}}}}",fs::read_to_string(file("panes-list")).unwrap());
            return ExitCode::SUCCESS;
        }
        if is(&["pane", "list"]) && file("panes-list").exists() {
            println!("{}", fs::read_to_string(file("panes-list")).unwrap());
            return ExitCode::SUCCESS;
        }
        if is(&["config", "check"]) && file("reject-config").exists() {
            return ExitCode::FAILURE;
        }
        if is(&["config", "check"]) || is(&["server", "reload-config"]) || is(&["stop"]) {
            return ExitCode::SUCCESS;
        }
        if is(&["rm", "--force"]) {
            let mut log = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(file("removed"))
                .unwrap();
            writeln!(log, "{}", args[2]).unwrap();
            if file("not-found").exists() {
                eprintln!("no such sandbox");
                return ExitCode::FAILURE;
            }
            if fs::read_to_string(file("fail-rm")).is_ok_and(|n| n == args[2]) {
                eprintln!("permission denied");
                return ExitCode::FAILURE;
            }
            return ExitCode::SUCCESS;
        }
    }
    if let Some(dir) = env::var_os("HERDR_TEST_START_DIR") {
        let dir = std::path::PathBuf::from(dir);
        let prefix = |a: &str, b: &str| {
            args.first().map(String::as_str) == Some(a)
                && args.get(1).map(String::as_str) == Some(b)
        };
        if prefix("pane", "split") {
            std::fs::write(dir.join("split-args"), args.join("\t")).unwrap();
            if let Ok(code) = std::fs::read_to_string(dir.join("split-error")) {
                eprintln!(
                    r#"{{"error":{{"code":"{code}","message":"split failed"}},"id":"cli:pane:split"}}"#
                );
                return ExitCode::FAILURE;
            }
            println!(r#"{{"result":{{"pane":{{"pane_id":"w1:p2"}}}}}}"#);
            return ExitCode::SUCCESS;
        }
        if prefix("tab", "create") {
            std::fs::write(dir.join("tab-args"), args.join("\t")).unwrap();
            println!(
                r#"{{"result":{{"root_pane":{{"pane_id":"w1:p2"}},"tab":{{"tab_id":"w1:t2"}}}}}}"#
            );
            return ExitCode::SUCCESS;
        }
        if prefix("pane", "run") {
            std::fs::write(dir.join("command"), &args[3]).unwrap();
            return ExitCode::SUCCESS;
        }
        if prefix("pane", "rename") || prefix("notification", "show") {
            return ExitCode::SUCCESS;
        }
        if args.first().is_some_and(|s| s == "create" || s == "exec") {
            if args.last().is_some_and(|arg| arg == "-l") {
                let file = std::fs::read_dir(dir.join("panes"))
                    .unwrap()
                    .map(|e| e.unwrap().path())
                    .find(|p| p.extension().is_some_and(|e| e == "json"))
                    .unwrap();
                std::fs::copy(file, dir.join("shell-snapshot")).unwrap();
            }
            use std::io::Write;
            let mut log = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("sbx-args"))
                .unwrap();
            writeln!(log, "{}", args.join("\t")).unwrap();
            if env::var("HERDR_TEST_SBX_FAILURE").as_deref() == Ok("create") && args[0] == "create"
            {
                eprintln!("permission denied");
                return ExitCode::FAILURE;
            }
            return ExitCode::SUCCESS;
        }
    }
    if args == ["version", "--json"] {
        if env::var("HERDR_TEST_SBX_FAILURE").as_deref() == Ok("version") {
            eprintln!("Not logged in");
            return ExitCode::FAILURE;
        }
        println!(r#"{{"client":{{"version":"0.40.1"}}}}"#);
    } else if args == ["daemon", "status", "--json"] {
        if env::var("HERDR_TEST_SBX_FAILURE").as_deref() == Ok("daemon") {
            eprintln!("offline");
            return ExitCode::FAILURE;
        }
        println!(r#"{{"running":true}}"#);
    } else if args == ["ls", "--json"] {
        if let Ok(flag) = env::var("HERDR_TEST_HANG_FLAG") {
            std::fs::write(flag, "started").unwrap();
            std::thread::sleep(std::time::Duration::from_secs(20));
        }
        println!(
            "{}",
            env::var("HERDR_TEST_INVENTORY").unwrap_or_else(|_| {
                r#"{"sandboxes":[{"name":"test-live","status":"stopped"}]}"#.into()
            })
        );
    } else if args == ["port", "ls", "test-live", "--json"]
        || args == ["ports", "test-live", "--json"]
    {
        if args[0] == "port" && env::var_os("HERDR_TEST_LEGACY_PORTS").is_some() {
            eprintln!("unknown command port");
            return ExitCode::FAILURE;
        }
        println!(
            "{}",
            env::var("HERDR_TEST_PORTS")
                .unwrap_or_else(|_| r#"{"ports":[{"mapping":"127.0.0.1:8080:3000/tcp"}]}"#.into())
        );
    } else if args == ["http://localhost:8080"] {
        return ExitCode::SUCCESS;
    } else if args.len() == 8
        && args[..6] == ["plugin", "pane", "open", "--plugin", "sbx.sandbox", "--entrypoint"]
        && args[7] == "--focus"
        && ["sandboxes", "config-editor"].contains(&args[6].as_str())
    {
        println!(r#"{{"result":{{"type":"plugin_pane_opened"}}}}"#);
    } else if args == ["pane", "get", "p1"] {
        println!(r#"{{"result":{{"pane":{{"pane_id":"p1"}}}}}}"#);
    } else if args == ["api","snapshot"] {
        println!(r#"{{"result":{{"snapshot":{{"workspaces":[{{"workspace_id":"w8","label":"info"}}],"panes":[{{"pane_id":"p1","workspace_id":"w8","label":"build"}}]}}}}}}"#);
    } else if args == ["pane", "list"] {
        println!(r#"{{"result":{{"panes":[{{"pane_id":"p1"}}]}}}}"#);
    } else {
        eprintln!("Unexpected sbx arguments: {args:?}");
        return ExitCode::from(2);
    }
    ExitCode::SUCCESS
}
