use crate::{
    list,
    management::{checked, Report},
    sbx::{self, Failure},
    start,
    state::{self, Mapping},
};
use serde_json::{json, Value};

pub fn run(map: &Mapping, row: &Value, sbx_bin: &str) -> Report {
    if row["workspaceMode"] != "clone" {
        return Err(Failure::new(
            "target",
            "The worktree is shared in mount mode. Fetch applies to clone mode.",
        ));
    }
    let path = row["localPath"]
        .as_str()
        .filter(|p| std::path::Path::new(p).is_absolute())
        .ok_or_else(|| Failure::new("target", "Mapping lacks an absolute local path."))?;
    let live = list::list_sandboxes(sbx_bin)?;
    let status = live.get(map.name);
    if status.is_none() || row["lifecycleState"] == "missing" {
        map.change(|r|{
            let mut r=r.filter(|r|r==row).ok_or_else(||Failure::new("conflict","Mapping changed while checking inventory."))?;
            if status.is_none(){r["lifecycleState"]=json!("missing");r["lastError"]=json!({"kind":"not-found","message":"The sandbox no longer exists.","at":state::now()});}
            else{r["lifecycleState"]=json!("created");r["lastError"]=Value::Null;}Ok(r)
        })?;
    }
    let status =
        status.ok_or_else(|| Failure::new("not-found", "The sandbox no longer exists."))?;
    let git = |args: &[&str]| {
        let mut cmd = vec!["-C", path];
        cmd.extend_from_slice(args);
        sbx::run("git", &cmd).map_err(|mut e| {
            if e.kind == "startup" {
                e.message = "Could not start git.".into();
            }
            e
        })
    };
    let repo = git(&["rev-parse", "--git-dir"])?;
    if !repo.status.success() {
        return Err(Failure {
            kind: "not-found",
            message: format!("Cannot open the Git repository at {path}."),
            output: repo.combined,
        });
    }
    let remote = format!("sandbox-{}", map.name);
    let mut fetched = None;
    if status
        .as_str()
        .is_some_and(|s| s.eq_ignore_ascii_case("running"))
        && git(&["remote", "get-url", &remote])?.status.success()
    {
        if let Ok(o) = git(&["fetch", "--verbose", &remote]) {
            if o.status.success() {
                fetched = Some(o.combined);
            }
        }
    }
    let (transport, output) = if let Some(out) = fetched {
        ("remote", out)
    } else {
        let guest = format!("/tmp/herdr-sbx-{}.bundle", state::nonce()?);
        let dir = tempfile::tempdir()?;
        let bundle = dir.path().join("clone.bundle");
        let host = start::path_text(&bundle)?;
        let outcome = (|| {
            checked(
                sbx_bin,
                &[
                    "exec",
                    map.name,
                    "--",
                    "git",
                    "-C",
                    if cfg!(windows) { "." } else { path },
                    "bundle",
                    "create",
                    &guest,
                    "--branches",
                ],
            )?;
            checked(sbx_bin, &["cp", &format!("{}:{guest}", map.name), host])?;
            checked(
                "git",
                &[
                    "-C",
                    path,
                    "fetch",
                    "--verbose",
                    host,
                    &format!("+refs/heads/*:refs/remotes/{remote}/*"),
                ],
            )
        })();
        if let Err(e) = checked(sbx_bin, &["exec", map.name, "--", "rm", "-f", &guest]) {
            eprintln!("Guest bundle cleanup failed: {}", e.message);
        }
        ("bundle", outcome?)
    };
    let prefix = format!("refs/remotes/{remote}/");
    let refs = git(&["for-each-ref", "--format=%(refname)", &prefix])?;
    let mut branches = vec![];
    let mut keep = vec![];
    let mut lines = vec![output.trim().to_owned()];
    for branch in refs
        .stdout
        .lines()
        .filter_map(|s| s.strip_prefix(&prefix))
        .filter(|s| *s != "HEAD")
    {
        let reference = format!("{remote}/{branch}");
        branches.push(reference.clone());
        let exists = |b: &str| {
            git(&[
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{b}"),
            ])
            .map(|o| o.status.success())
        };
        let (local, reason) = if exists(branch)? {
            if !git(&[
                "branch",
                "--contains",
                &reference,
                "--format=%(refname:short)",
            ])?
            .stdout
            .trim()
            .is_empty()
            {
                continue;
            }
            let mut name = format!("{branch}-sandbox");
            let mut n = 2;
            while exists(&name)? {
                name = format!("{branch}-sandbox-{n}");
                n += 1;
            }
            (
                name,
                format!("the host's {branch} does not contain these commits"),
            )
        } else {
            (branch.to_owned(), "the host has no such branch".to_owned())
        };
        let command = format!(
            "git -C {} branch -- {} {}",
            start::quote(path, cfg!(windows))?,
            start::quote(&local, cfg!(windows))?,
            start::quote(&reference, cfg!(windows))?
        );
        lines.push(command.clone());
        keep.push(json!({"ref":reference,"local":local,"reason":reason,"command":command}));
    }
    Ok((
        json!({"ok":true,"paneId":map.pane,"remote":remote,"transport":transport,"branches":branches,"keep":keep}),
        lines,
    ))
}
