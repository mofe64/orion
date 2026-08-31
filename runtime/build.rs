use std::env;
use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=ORION_BUILD_REVISION");
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/index");

    let revision = env::var("ORION_BUILD_REVISION")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(git_revision)
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=ORION_BUILD_REVISION={revision}");
}

fn git_revision() -> Option<String> {
    let manifest_directory = env::var("CARGO_MANIFEST_DIR").ok()?;
    let root = Path::new(&manifest_directory).parent()?;
    let output = Command::new("git")
        .args(["-C", root.to_str()?, "rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|value| value.trim().to_owned())
}
