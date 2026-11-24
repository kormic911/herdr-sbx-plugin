use regex::Regex;
use serde_json::Value;
use std::{env, fs, io, path::Path};

pub struct Config {
    pub values: Value,
    pub sbx: String,
}

fn text(v: &Value) -> bool {
    v.as_str().is_some_and(|s| !s.is_empty())
}

fn strings(v: &Value, nonempty: bool) -> bool {
    v.as_array().is_some_and(|a| {
        a.iter()
            .all(|v| v.as_str().is_some_and(|s| !nonempty || !s.is_empty()))
    })
}

impl Config {
    pub fn load(dir: &Path) -> Result<Self, String> {
        let path = dir.join("config.json");
        let text = Self::read(&path)?;
        Self::from_raw(&Self::parse(text.as_deref())?)
    }

    pub fn read(path: &Path) -> Result<Option<String>, String> {
        match fs::read_to_string(path) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("Could not read {}: {e}", path.display())),
        }
    }

    pub fn parse(text: Option<&str>) -> Result<Value, String> {
        match text {
            Some(s) if !s.trim().is_empty() => {
                serde_json::from_str(s).map_err(|e| format!("config.json is not valid JSON: {e}"))
            }
            _ => Ok(serde_json::json!({})),
        }
    }

    pub fn defaults() -> Value {
        serde_json::from_str(include_str!("config-defaults.json")).expect("valid built-in defaults")
    }

    pub fn from_raw(raw: &Value) -> Result<Self, String> {
        let mut values = Self::defaults();
        let env_entry =
            Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*(=[^\r\n\x{2028}\x{2029}]*)?$").unwrap();
        let prefix = Regex::new(r"^[A-Za-z0-9][A-Za-z0-9-]*$").unwrap();
        for (key, v) in raw
            .as_object()
            .ok_or("config.json must contain a JSON object.")?
        {
            let valid = match key.as_str() {
                "agentKind" | "shell" => text(v),
                "template" | "memory" | "sbxBin" => v.is_null() || text(v),
                "cpus" => v.is_null() || v.as_f64().is_some_and(|n| n > 0. && n.fract() == 0.),
                "paneRatio" => v.as_f64().is_some_and(|n| n > 0. && n < 1.),
                "workspaceMode" => matches!(v.as_str(), Some("mount" | "clone")),
                "paneDirection" => matches!(v.as_str(), Some("right" | "down")),
                "openIn" => matches!(v.as_str(), Some("tab" | "split")),
                "reportAgentStatus" | "cleanupOnWorktreeRemoved" => v.is_boolean(),
                "sandboxNamePrefix" => v.as_str().is_some_and(|s| prefix.is_match(s)),
                "agentArgs" => v
                    .as_object()
                    .is_some_and(|m| m.values().all(|a| strings(a, true))),
                "customAgents" => v.is_object(),
                "env" | "agentEnv" => v.as_array().is_some_and(|a| {
                    a.iter()
                        .all(|v| v.as_str().is_some_and(|s| env_entry.is_match(s)))
                }),
                "kits" | "kitArgs" | "envFiles" | "publish" | "denyNetwork" | "extraWorkspaces" => {
                    strings(v, true)
                }
                _ => return Err(format!("Unknown config key(s): {key}.")),
            };
            if !valid {
                return Err(format!("Invalid value for config key \"{key}\"."));
            }
            values[key] = v.clone();
        }
        let sbx = values["sbxBin"]
            .as_str()
            .map(String::from)
            .or_else(|| env::var("HERDR_SBX_BIN").ok().filter(|bin| !bin.is_empty()))
            .unwrap_or_else(|| "sbx".into());
        Ok(Self { values, sbx })
    }

    pub fn builtins() -> &'static Value {
        static BUILTINS: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| {
            serde_json::from_str(include_str!("agents.json")).expect("valid built-in agents")
        });
        &BUILTINS
    }
    pub fn agent(&self, kind: &str) -> Result<(&Value, Vec<&str>), String> {
        // The built-ins and defaults are embedded in the binary for standalone installs.
        let custom = self.values["customAgents"].as_object().unwrap();
        let kind_pattern = Regex::new(r"^[a-z0-9][a-z0-9-]*$").unwrap();
        for (kind, profile) in custom {
            let valid = kind_pattern.is_match(kind)
                && profile.as_object().is_some_and(|m| {
                    m.keys().all(|k| {
                        matches!(
                            k.as_str(),
                            "title"
                                | "command"
                                | "sbxAgent"
                                | "defaultArgs"
                                | "herdrDetectionKind"
                                | "setupScript"
                        )
                    })
                })
                && text(&profile["title"])
                && text(&profile["sbxAgent"])
                && strings(&profile["command"], false)
                && !profile["command"].as_array().unwrap().is_empty()
                && profile.get("defaultArgs").is_none_or(|a| strings(a, false))
                && (profile["herdrDetectionKind"].is_null()
                    || text(&profile["herdrDetectionKind"]))
                && (profile["setupScript"].is_null() || profile["setupScript"].is_string());
            if !valid {
                return Err(format!("Invalid customAgents.{kind} profile."));
            }
        }
        let profile = custom
            .get(kind)
            .or_else(|| Self::builtins().get(kind))
            .ok_or_else(|| format!("Unknown agentKind \"{kind}\"."))?;
        let args = self.values["agentArgs"]
            .get(kind)
            .or_else(|| profile.get("defaultArgs"));
        Ok((
            profile,
            profile["command"]
                .as_array()
                .unwrap()
                .iter()
                .chain(args.and_then(Value::as_array).into_iter().flatten())
                .map(|v| v.as_str().unwrap())
                .collect(),
        ))
    }
}
