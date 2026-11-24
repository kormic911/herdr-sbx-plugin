use crate::{management::Report, sbx::Failure, start, state};
use serde_json::json;
use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
};

fn write(path: &Path, text: &str) -> Result<(), Failure> {
    let mut tmp = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    if let Ok(meta) = fs::metadata(path) {
        tmp.as_file().set_permissions(meta.permissions())?;
    }
    tmp.write_all(text.as_bytes())?;
    tmp.flush()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

pub(crate) fn config_path() -> Option<PathBuf> {
    env::var_os("HERDR_CONFIG_PATH")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            let base = env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| {
                    if cfg!(windows) {
                        env::var_os("APPDATA").map(PathBuf::from).or_else(|| {
                            env::var_os("USERPROFILE")
                                .map(|p| PathBuf::from(p).join("AppData/Roaming"))
                        })
                    } else {
                        None
                    }
                })
                .or_else(|| env::home_dir().map(|h| h.join(".config")))?;
            Some(base.join("herdr/config.toml"))
        })
}

pub fn run(bin: &str) -> Report {
    let config =
        config_path().ok_or_else(|| Failure::new("config", "Could not locate Herdr config."))?;
    let path = std::path::absolute(config)?;
    fs::create_dir_all(path.parent().unwrap())?;
    let _lock = state::lock(path.with_added_extension("sbx.lock"))?;
    let before = match fs::read_to_string(&path) {
        Ok(s) => Some(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let original = before.as_deref().unwrap_or_default();
    let doc = original
        .trim_start_matches('\u{feff}')
        .parse::<toml::Table>()
        .map_err(|e| Failure::new("config", e.to_string()))?;
    let entries = doc
        .get("keys")
        .and_then(|k| k.get("command"))
        .and_then(|c| c.as_array())
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let (mut added, mut existing, mut warnings) = (vec![], vec![], vec![]);
    let mut text = original.to_owned();
    for (letter, action, description) in [
        ("a", "start-agent", "start an agent in a Docker Sandbox"),
        (
            "b",
            "reconnect",
            "bring the agent back in its Docker Sandbox",
        ),
        ("s", "open-shell", "open a shell in the Docker Sandbox"),
        ("o", "sandboxes", "show the Docker Sandboxes overlay"),
        ("c", "edit-config", "edit Docker Sandbox settings"),
    ] {
        let key = format!("prefix+shift+{letter}");
        let command = format!("sbx.sandbox.{action}");
        if let Some(entry) = entries
            .iter()
            .find(|e| e.get("command").and_then(|v| v.as_str()) == Some(&command))
        {
            let old = entry.get("key").and_then(|v| v.as_str()).unwrap_or("");
            existing.push(json!({"action":action,"key":old}));
            if "prefix+shift+d prefix+shift+g prefix+shift+h prefix+shift+j prefix+shift+k prefix+shift+l prefix+shift+n prefix+shift+p prefix+shift+r prefix+shift+t prefix+shift+tab prefix+shift+w prefix+shift+x prefix+b prefix+c prefix+e prefix+g prefix+h prefix+j prefix+k prefix+l prefix+n prefix+o prefix+p prefix+q prefix+r prefix+s prefix+v prefix+w prefix+x prefix+z prefix+tab prefix+minus prefix+? prefix+[".split_whitespace().any(|s|s==old){warnings.push(format!("{command} uses Herdr's reserved chord {old}."));}
            continue;
        }
        if entries
            .iter()
            .any(|e| e.get("key").and_then(|v| v.as_str()) == Some(&key))
        {
            warnings.push(format!(
                "{key} is already bound. Assign {command} a free chord."
            ));
            continue;
        }
        text.push_str(&format!("\n[[keys.command]]\nkey = \"{key}\"\ntype = \"plugin_action\"\ncommand = \"{command}\"\ndescription = \"{description}\"\n"));
        added.push(json!({"key":key,"action":action}));
    }
    if text != original {
        write(&path, &text)?;
    }
    let mut reloaded = false;
    match start::herdr(bin, &["config", "check"]) {
        Ok(_) => {
            start::herdr(bin, &["server", "reload-config"])?;
            reloaded = true;
        }
        Err(e) if e.kind == "startup" => warnings.push(
            "Run herdr config check and herdr server reload-config when Herdr is available.".into(),
        ),
        Err(e) => {
            if text != original && fs::read_to_string(&path)? == text {
                if before.is_some() {
                    write(&path, original)?;
                } else {
                    fs::remove_file(&path)?;
                }
            }
            return Err(e);
        }
    }
    Ok((
        json!({"ok":true,"configPath":path,"added":added,"existing":existing,"warnings":warnings,"reloaded":reloaded}),
        warnings,
    ))
}
