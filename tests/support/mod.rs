use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub fn compile_cli(root: &Path) -> PathBuf {
    let fake = root.join(format!("fake sbx{}", std::env::consts::EXE_SUFFIX));
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sbx.rs");
    let compiler = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let build = Command::new(compiler)
        .arg(source)
        .arg("-o")
        .arg(&fake)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    fake
}
