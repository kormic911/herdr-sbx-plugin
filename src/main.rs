mod action;
mod bindings;
mod config;
mod confirmation;
mod doctor;
mod editor;
mod fetch;
mod inventory;
mod list;
mod location;
mod management;
mod mapped;
mod overlay;
mod ports;
mod run_action;
mod sbx;
mod screen;
mod shell;
mod start;
mod state;
mod theme;

use std::{env, process::ExitCode};

fn main() -> ExitCode {
    let mode = env::args().nth(1);
    let mode = if mode.as_deref() == Some("action") {
        let id = env::var("HERDR_PLUGIN_ACTION_ID").unwrap_or_default();
        let command = if id == "sandboxes" {
            "open-overlay"
        } else {
            &id
        };
        #[cfg(windows)]
        {
            // Keep long-running bridges on the staged copy so Cargo can rebuild.
            return match std::process::Command::new("powershell.exe")
                .args([
                    "-NoLogo",
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                    "bin/native-action.ps1",
                    command,
                ])
                .status()
            {
                Ok(status) => ExitCode::from(status.code().unwrap_or(1) as u8),
                Err(error) => ExitCode::from(action::emit(
                    &id,
                    Err(sbx::Failure::new("startup", error.to_string())),
                )),
            };
        }
        #[cfg(not(windows))]
        {
            Some(command.to_owned())
        }
    } else {
        mode
    };
    match mode.as_deref() {
        Some("worktree-removed") => match management::events::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!(
                    "Worktree cleanup failed: {} {}",
                    error.message,
                    error.output.trim()
                );
                ExitCode::FAILURE
            }
        },
        Some("sandboxes") => {
            ExitCode::from(overlay::run(env::args().skip(2).any(|arg| arg == "--once")))
        }
        Some("open-shell") => ExitCode::from(action::emit(
            "open-shell",
            shell::open(&env::args().skip(2).collect::<Vec<_>>()),
        )),
        Some("start-agent") => ExitCode::from(action::emit("start-agent", start::run())),
        Some(mode @ ("bridge-start" | "bridge-shell")) => match (if mode == "bridge-shell" {
            shell::bridge
        } else {
            start::bridge
        })(
            &env::args().skip(2).collect::<Vec<_>>(),
        ) {
            Ok(code) => ExitCode::from(code as u8),
            Err(error) => {
                eprintln!("{}\n{}", error.message, error.output.trim());
                ExitCode::FAILURE
            }
        },
        Some(
            mode @ ("reconnect"
            | "stop"
            | "fetch-changes"
            | "forget-mapping"
            | "replace-sandbox"
            | "prune-mappings"
            | "install-keybindings"),
        ) => ExitCode::from(action::emit(mode, management::run(mode))),
        Some("info") => ExitCode::from(action::emit("info", mapped::info())),
        Some("open-port") => ExitCode::from(action::emit("open-port", mapped::open_port())),
        Some("open-overlay") => ExitCode::from(action::emit("sandboxes", mapped::open_overlay())),
        Some("edit-config") => ExitCode::from(action::emit(
            "edit-config",
            mapped::open_pane(if cfg!(windows) {
                "config-editor-windows"
            } else {
                "config-editor"
            }),
        )),
        Some("config-editor") => match editor::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("Cannot edit plugin config: {e}");
                ExitCode::FAILURE
            }
        },
        Some("doctor") => ExitCode::from(action::emit("doctor", doctor::inspect())),
        Some("list-sandboxes") => ExitCode::from(action::emit("list-sandboxes", list::run())),
        Some("deletion-confirmation") => match confirmation::run() {
            Ok(code) => ExitCode::from(code),
            Err(error) => {
                eprintln!("Cannot complete sandbox confirmation: {error}");
                ExitCode::FAILURE
            }
        },
        Some("run-action") => match run_action::run(&env::args().skip(2).collect::<Vec<_>>()) {
            Ok(code) => ExitCode::from(code),
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(2)
            }
        },
        Some("--version") => {
            println!("herdr-sbx {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!(
                "Usage: herdr-sbx run-action ACTION [seconds] | open-shell [--sandbox NAME] | start-agent | doctor | info | open-port | open-overlay | list-sandboxes | sandboxes [--once] | deletion-confirmation | worktree-removed | --version"
            );
            ExitCode::from(2)
        }
    }
}
