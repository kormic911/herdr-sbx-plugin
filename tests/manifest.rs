use serde_json::Value;
use std::path::Path;

#[test]
fn manifest_and_readme_match_the_native_plugin() {
    let manifest = include_str!("../herdr-plugin.toml")
        .parse::<toml::Table>()
        .unwrap();
    let readme = include_str!("../README.md");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert_eq!(
        manifest["version"].as_str(),
        Some(env!("CARGO_PKG_VERSION"))
    );
    for action in manifest["actions"].as_array().unwrap() {
        let id = action["id"].as_str().unwrap();
        assert!(readme.contains(&format!("| `{id}` |")), "{id}");
        let command = action["command"].as_array().unwrap();
        assert_eq!(command[0].as_str(), Some("./target/release/herdr-sbx"));
        assert_eq!(command[1].as_str(), Some("action"));
    }
    for kind in ["build", "panes", "events"] {
        for entry in manifest[kind].as_array().unwrap() {
            for arg in entry["command"].as_array().unwrap() {
                let arg = arg.as_str().unwrap();
                if arg.starts_with("bin/") || arg.starts_with("scripts/") {
                    assert!(root.join(arg).is_file(), "Missing {arg}");
                }
            }
        }
    }
    let defaults: Value =
        serde_json::from_str(include_str!("../src/config-defaults.json")).unwrap();
    for key in defaults.as_object().unwrap().keys() {
        assert!(readme.contains(&format!("| `{key}` |")), "{key}");
    }
}
