use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet},
    fs, io,
    path::Path,
};

pub fn mappings(state: &Path) -> Result<Vec<(String, Value)>, String> {
    let entries = match fs::read_dir(state.join("panes")) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    };
    let mut paths = entries
        .map(|e| e.map(|f| f.path()))
        .collect::<io::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    paths.sort();
    let mut positions = BTreeMap::new();
    let mut rows = Vec::new();
    for path in paths
        .into_iter()
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
    {
        let raw = match fs::read(&path) {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let row: Value =
            serde_json::from_slice(&raw).map_err(|e| format!("{}: {e}", path.display()))?;
        let id = row["paneId"]
            .as_str()
            .filter(|_| row["version"] == 1)
            .ok_or_else(|| format!("Unsupported mapping: {}", path.display()))?;
        let index = *positions.entry(id.to_owned()).or_insert(rows.len());
        let entry = (id.to_owned(), row);
        if index == rows.len() {
            rows.push(entry);
        } else {
            rows[index] = entry;
        }
    }
    Ok(rows)
}

pub fn pane_ids(value: &Value) -> Result<HashSet<String>, String> {
    let rows = value
        .pointer("/result/panes")
        .filter(|p| !p.is_null())
        .or_else(|| value.get("panes"))
        .and_then(Value::as_array)
        .ok_or("herdr pane list did not return a pane list.")?;
    rows.iter()
        .map(|p| {
            p["pane_id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(String::from)
                .ok_or_else(|| "Herdr returned a pane without an ID.".to_string())
        })
        .collect()
}

fn array<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a [Value]> {
    let inner = keys.iter().find_map(|key| value.get(key)).unwrap_or(value);
    if inner.is_null() || value.as_object().is_some_and(|m| m.is_empty()) {
        Some(&[])
    } else {
        inner.as_array().map(Vec::as_slice)
    }
}

fn sandbox_fields<'a>(row: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .find_map(|k| row.get(k).filter(|v| !v.is_null()))
}

pub fn sandboxes(value: &Value) -> Result<BTreeMap<String, Value>, String> {
    let mut result = BTreeMap::new();
    let mut insert = |name: Option<&str>, row: &Value| -> Result<(), String> {
        let name = name
            .filter(|s| !s.is_empty())
            .ok_or("sandbox entry without a recognizable name")?;
        let status = sandbox_fields(row, &["status", "state", "Status"])
            .cloned()
            .unwrap_or(Value::Null);
        result.insert(name.into(), status);
        Ok(())
    };
    if let Some(rows) = array(value, &["sandboxes", "items", "Sandboxes"]) {
        for row in rows {
            insert(
                sandbox_fields(row, &["name", "Name", "id"]).and_then(Value::as_str),
                row,
            )?;
        }
    } else {
        for (key, row) in value.as_object().ok_or("unrecognized sandbox inventory")? {
            let fields = [
                "name",
                "Name",
                "id",
                "status",
                "state",
                "Status",
                "agent",
                "workspaces",
                "created_at",
            ];
            if !fields.iter().any(|field| row.get(field).is_some()) {
                return Err("unrecognized sandbox inventory".into());
            }
            let name =
                sandbox_fields(row, &["name", "Name"]).map_or(Some(key.as_str()), Value::as_str);
            insert(name, row)?;
        }
    }
    Ok(result)
}

#[derive(Default)]
struct Spec {
    single: Option<u16>,
    host: Option<u16>,
    guest: Option<u16>,
}

fn spec(v: &Value) -> Spec {
    if let Some(n) = v.as_u64() {
        return Spec {
            single: u16::try_from(n).ok().filter(|p| *p > 0),
            ..Spec::default()
        };
    }
    let Some(raw) = v.as_str() else {
        return Spec::default();
    };
    let raw = raw.trim();
    let raw = raw
        .rsplit_once('/')
        .filter(|(_, s)| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric()))
        .map_or(raw, |(s, _)| s);
    let parts: Vec<_> = raw.split(':').collect();
    let number = |s: &str| {
        s.bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| s.parse::<u16>().ok())
            .flatten()
            .filter(|n| *n > 0)
    };
    match parts.as_slice() {
        [s] => Spec {
            single: number(s),
            ..Spec::default()
        },
        [ip, s] if ip.is_empty() || !ip.bytes().all(|b| b.is_ascii_digit()) => Spec {
            single: number(s),
            ..Spec::default()
        },
        [h, g] | [_, h, g] => Spec {
            host: number(h),
            guest: number(g),
            single: None,
        },
        _ => Spec::default(),
    }
}

pub struct PublishedPort<'a> {
    pub host: Option<u16>,
    pub guest: u16,
    pub raw: &'a Value,
}

pub fn ports(value: &Value) -> Result<Vec<(Option<u16>, u16)>, String> {
    Ok(published_ports(value)?
        .iter()
        .map(|p| (p.host, p.guest))
        .collect())
}

pub fn published_ports(value: &Value) -> Result<Vec<PublishedPort<'_>>, String> {
    let rows =
        array(value, &["ports", "items", "published"]).ok_or("unrecognized port inventory")?;
    let host = [
        "host_port",
        "hostPort",
        "host",
        "published",
        "local_port",
        "localPort",
    ];
    let guest = [
        "sandbox_port",
        "sandboxPort",
        "container_port",
        "containerPort",
        "target",
        "port",
    ];
    let mut result = Vec::new();
    for row in rows {
        let combined = ["spec", "mapping", "publish"]
            .iter()
            .chain(&host)
            .map(|k| spec(&row[*k]))
            .find(|p| p.guest.is_some())
            .unwrap_or_default();
        let h = host
            .iter()
            .map(|k| spec(&row[*k]))
            .find_map(|p| p.single.or(p.host))
            .or(combined.host);
        let g = guest
            .iter()
            .map(|k| spec(&row[*k]))
            .find_map(|p| p.single.or(p.guest))
            .or(combined.guest);
        if let Some(g) = g {
            result.push(PublishedPort {
                host: h,
                guest: g,
                raw: row,
            });
        }
    }
    if !rows.is_empty() && result.is_empty() {
        return Err("no recognizable sandbox ports".into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_the_supported_sandbox_inventory_shapes() {
        for empty in [Value::Null, json!([]), json!({}), json!({"sandboxes":null})] {
            assert!(sandboxes(&empty).unwrap().is_empty());
        }
        for input in [
            json!({"items":[{"Name":"demo","state":"ready"}]}),
            json!({"demo":{"Status":"ready"}}),
        ] {
            assert_eq!(sandboxes(&input).unwrap().get("demo").unwrap(), "ready");
        }
        assert!(sandboxes(&json!({"unexpected":[]})).is_err());
        assert!(sandboxes(&json!([{"name":"ok"},{}])).is_err());
    }

    #[test]
    fn parses_numeric_address_and_mapping_port_fields() {
        let input = json!({"ports":[{"host_port":8080,"sandbox_port":3000},{"hostPort":"9000","containerPort":"80"},{"published":"127.0.0.1:8443","target":443},{"published":"8081:3001/tcp"},{"spec":"0.0.0.0:8082:3002/udp"},{"port":22}]});
        assert_eq!(
            ports(&input).unwrap(),
            [
                (Some(8080), 3000),
                (Some(9000), 80),
                (Some(8443), 443),
                (Some(8081), 3001),
                (Some(8082), 3002),
                (None, 22)
            ]
        );
        assert!(ports(&json!({"items":null})).unwrap().is_empty());
        assert!(ports(&json!([{"port":70000}])).is_err());
    }
}
