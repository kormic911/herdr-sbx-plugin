use serde_json::Value;

pub struct Location {
    pub workspace: String,
    pub pane: String,
}

// Herdr encodes its public pane number with a one-based, 32-character alphabet.
fn public_number(code: &str) -> Option<usize> {
    const DIGITS: &[u8] = b"123456789ABCDEFGHJKMNPQRSTVWXYZ0";
    if code.is_empty() {
        return None;
    }
    code.bytes().try_fold(0usize, |n, c| {
        let digit = DIGITS.iter().position(|&d| d == c)? + 1;
        n.checked_mul(DIGITS.len())?.checked_add(digit)
    })
}

impl Location {
    pub fn resolve(snapshot: &Value, id: &str, saved_workspace: Option<&str>) -> Self {
        let find = |list: &str, key: &str, id: &str| {
            snapshot[list].as_array()?.iter().find(|v| v[key] == id)
        };
        let live = find("panes", "pane_id", id);
        let parts = id.rsplit_once(":p");
        let workspace_id = live
            .and_then(|v| v["workspace_id"].as_str())
            .or(parts.map(|(ws, _)| ws))
            .or(saved_workspace);
        let ws = workspace_id.and_then(|id| find("workspaces", "workspace_id", id));
        let workspace = ws
            .and_then(|v| v["label"].as_str())
            .filter(|s| !s.trim().is_empty())
            .map(str::to_owned)
            .or_else(|| ws.and_then(|v| v["number"].as_u64()).map(|n| n.to_string()))
            .or_else(|| {
                workspace_id
                    .and_then(|id| public_number(id.strip_prefix('w')?))
                    .map(|n| n.to_string())
            })
            .unwrap_or_else(|| workspace_id.unwrap_or("Unknown").into());
        let number = parts
            .and_then(|(_, code)| public_number(code))
            .map(|n| n.to_string());
        let label = live
            .and_then(|v| v["label"].as_str())
            .filter(|s| !s.trim().is_empty());
        let pane = match (number, label) {
            (Some(n), Some(name)) if n != name => format!("{n} · {name}"),
            (Some(n), _) => n,
            (None, Some(name)) => name.into(),
            _ => id.into(),
        };
        Self { workspace, pane }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn shows_herdr_labels_and_public_numbers_for_live_and_closed_panes() {
        let snapshot = json!({"workspaces":[{"workspace_id":"w8","label":"info"}],"panes":[{"pane_id":"w8:p3","workspace_id":"w8","label":"build"}]});
        let live = Location::resolve(&snapshot, "w8:p3", Some("w1"));
        assert_eq!(
            (live.workspace.as_str(), live.pane.as_str()),
            ("info", "3 · build")
        );
        let closed = Location::resolve(&snapshot, "w8:p11", None);
        assert_eq!(
            (closed.workspace.as_str(), closed.pane.as_str()),
            ("info", "33")
        );
        assert_eq!(Location::resolve(&Value::Null, "w8:pA", None).pane, "10");
    }
}
