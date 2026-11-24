use crate::{
    action,
    config::Config,
    inventory, mapped, ports,
    sbx::Failure,
    start,
    state::{self, Mapping},
};
use serde_json::{json, Value};
use std::{env, path::Path};

fn names(row: &Value) -> Vec<&str> {
    row["sandboxName"]
        .as_str()
        .into_iter()
        .chain(
            row["replacesSandboxNames"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str),
        )
        .collect()
}
fn conflict(message: &str) -> Failure {
    Failure::new("conflict", message)
}

pub fn open(options: &[String]) -> Result<(Value, Vec<String>), Failure> {
    let (dir, config_dir, config, ctx) = action::context()?;
    let rows = inventory::mappings(&dir).map_err(|e| Failure::new("startup", e))?;
    let row = match options {
        [] => mapped::focused(&rows, &ctx)?,
        [flag, name] if flag == "--sandbox" => {
            let mut matches = rows
                .iter()
                .filter(|(_, row)| row["sandboxName"] == name.as_str());
            let (_, row) = matches.next().ok_or_else(|| {
                Failure::new("target", format!("No mapping tracks sandbox {name}."))
            })?;
            if matches.next().is_some() {
                return Err(Failure::new(
                    "target",
                    format!("Multiple panes track sandbox {name}. Focus its mapped pane."),
                ));
            }
            row.clone()
        }
        _ => return Err(Failure::new("target", "Usage: open-shell [--sandbox NAME]")),
    };
    let pane = row["paneId"].as_str().unwrap();
    let name = row["sandboxName"]
        .as_str()
        .filter(|n| ports::valid_name(n))
        .ok_or_else(|| Failure::new("target", "Mapping has no valid sandbox name."))?;
    let bin = env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into());
    let anchor = mapped::field(&ctx, "focused_pane_id", "HERDR_PANE_ID");
    let cwd = ["workdir", "localPath"]
        .iter()
        .find_map(|k| row[k].as_str().filter(|p| Path::new(p).is_dir()));
    let mut split = vec!["pane", "split"];
    split.extend(anchor.as_deref().filter(|id| !id.is_empty()));
    split.extend(["--direction", "down", "--ratio", "0.5", "--focus"]);
    if let Some(cwd) = cwd {
        split.extend(["--cwd", cwd]);
    }
    let (reply, pointer) = match start::herdr(&bin, &split) {
        Ok(reply) => (reply, "/pane/pane_id"),
        Err(e) => {
            // Retry only a definite rejection: a timeout could follow a successful split.
            let missing = e
                .output
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .any(|v| {
                    v.pointer("/error/code").and_then(Value::as_str) == Some("pane_not_found")
                });
            if !missing {
                return Err(e);
            }
            let mut tab = vec!["tab", "create", "--focus"];
            if let Some(cwd) = cwd {
                tab.extend(["--cwd", cwd]);
            }
            eprintln!("The target pane is closed. Opening the sandbox shell in a new tab.");
            (start::herdr(&bin, &tab)?, "/root_pane/pane_id")
        }
    };
    let shell = reply
        .get("result")
        .unwrap_or(&reply)
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Failure::new("unknown", "Herdr did not return the shell pane ID."))?;
    let exe = start::ordinary_path(env::current_exe()?)?;
    let command = start::pane_command(
        &[
            start::path_text(&exe)?,
            "bridge-shell",
            "--pane-id",
            pane,
            "--shell-pane-id",
            shell,
            "--state-dir",
            start::path_text(&dir)?,
            "--config-dir",
            start::path_text(&config_dir)?,
            "--name",
            name,
            "--sbx-bin",
            &config.sbx,
        ],
        None,
    )?;
    start::herdr(
        &bin,
        &["pane", "rename", shell, &format!("sbx shell {name}")],
    )?;
    start::herdr(&bin, &["pane", "run", shell, &command])?;
    Ok((
        json!({"ok":true,"paneId":shell,"mappedPaneId":pane,"sandboxName":name}),
        vec![],
    ))
}

pub fn bridge(args: &[String]) -> Result<i32, Failure> {
    let option = |flag: &str| {
        args.chunks_exact(2)
            .find(|p| p[0] == flag)
            .map(|p| p[1].as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Failure::new("startup", format!("Missing {flag}")))
    };
    let map = Mapping {
        dir: Path::new(option("--state-dir")?),
        pane: option("--pane-id")?,
        name: option("--name")?,
    };
    let shell_pane = option("--shell-pane-id")?;
    let sbx = option("--sbx-bin")?;
    let config =
        Config::load(Path::new(option("--config-dir")?)).map_err(|e| Failure::new("config", e))?;
    let read_rows = || inventory::mappings(map.dir).map_err(|e| Failure::new("startup", e));
    let rows = read_rows()?;
    let (_, snapshot) = rows
        .iter()
        .find(|(id, _)| id == map.pane)
        .ok_or_else(|| conflict("The mapping disappeared."))?;
    let locked = names(snapshot);
    let pid = std::process::id();
    let row = map.change_with_names(&locked, |old| {
        let mut row = old.ok_or_else(|| conflict("The mapping disappeared."))?;
        let mine = names(&row);
        if row["sandboxName"] != map.name
            || !ports::valid_name(map.name)
            || !mine.iter().all(|n| locked.contains(n))
        {
            return Err(conflict("The mapping changed before the shell opened."));
        }
        if matches!(
            row["lifecycleState"].as_str(),
            Some("provisional" | "missing")
        ) {
            return Err(Failure::new(
                "target",
                "The mapped sandbox does not exist yet.",
            ));
        }
        for (_, other) in read_rows()? {
            if names(&other).iter().any(|n| mine.contains(n))
                && state::live_owner(&other["deletingPid"], &other["deletingToken"])
            {
                return Err(conflict("A deletion owns this sandbox."));
            }
        }
        let mut shells = row["shellPids"].as_array().cloned().unwrap_or_default();
        shells.retain(|s| s["pid"] != pid && state::live_owner(&s["pid"], &s["token"]));
        shells.push(json!({
            "pid": pid, "token": state::own_token(), "since": state::now(),
            "paneId": map.pane, "shellPaneId": shell_pane
        }));
        row["shellPids"] = json!(shells);
        Ok(row)
    })?;
    let result = start::exec_args(
        &row,
        &config.values,
        &[config.values["shell"].as_str().unwrap(), "-l"],
        true,
        false,
    )
    .and_then(|args| Ok(start::interactive(sbx, args)?.code().unwrap_or(1)));
    if let Err(e) = map.change(|old| {
        let mut row = old.ok_or_else(|| conflict("The mapping disappeared."))?;
        if row["sandboxName"] != map.name {
            return Err(conflict("The mapping was replaced."));
        }
        let shells = row["shellPids"]
            .as_array_mut()
            .ok_or_else(|| conflict("The shell record moved."))?;
        shells.retain(|s| s["pid"] != pid);
        Ok(row)
    }) {
        eprintln!("Could not release shell record: {}", e.message);
    }
    result
}
