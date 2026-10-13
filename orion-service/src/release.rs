use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub struct InstalledRelease {
    pub root: PathBuf,
    pub revision: String,
}

#[derive(Deserialize)]
struct Metadata {
    revision: String,
}

impl InstalledRelease {
    pub fn discover() -> Result<Self, String> {
        let executable = std::env::current_exe()
            .map_err(|e| format!("Could not locate the running Orion service: {e}"))?;
        Self::from_executable(&executable)
    }

    fn from_executable(executable: &Path) -> Result<Self, String> {
        let executable = executable.canonicalize().map_err(|e| {
            format!(
                "Could not locate Orion executable {}: {e}",
                executable.display()
            )
        })?;
        let root = executable
            .parent()
            .into_iter()
            .flat_map(Path::ancestors)
            .find(|path| path.join("release.json").exists())
            .ok_or_else(|| {
                format!(
                    "No installed Orion release found above {} (missing release.json)",
                    executable.display()
                )
            })?;
        let manifest = root.join("release.json");
        let metadata: Metadata = serde_json::from_slice(
            &std::fs::read(&manifest)
                .map_err(|e| format!("Could not read {}: {e}", manifest.display()))?,
        )
        .map_err(|e| {
            format!(
                "Invalid Orion release metadata at {}: {e}",
                manifest.display()
            )
        })?;
        if metadata.revision.trim().is_empty() {
            return Err(format!("Missing revision in {}", manifest.display()));
        }
        Ok(Self {
            root: root.to_path_buf(),
            revision: metadata.revision,
        })
    }

    pub fn python(&self) -> PathBuf {
        // Do not canonicalize this symlink: Python uses the venv path to find its packages.
        self.root.join("speech/.venv/bin/python")
    }

    pub fn check_speech(&self) -> Result<(), String> {
        let python = self.python();
        if !python.is_file() {
            return Err(format!(
                "Missing speech Python environment: {}",
                python.display()
            ));
        }
        // Locate the installed module without importing inference providers or loading models.
        let script = "import importlib.util, pathlib, sys\n\
            expected = pathlib.Path(sys.argv[1]).resolve()\n\
            assert pathlib.Path(sys.prefix).resolve() == expected, 'Python is not using the release environment'\n\
            spec = importlib.util.find_spec('orion_speech_worker.worker')\n\
            assert spec and spec.origin and pathlib.Path(spec.origin).resolve().is_relative_to(expected), 'Speech worker is not installed in the release environment'";
        let result = Command::new(&python)
            .args(["-I", "-c", script])
            .arg(self.root.join("speech/.venv"))
            .current_dir(&self.root)
            .output()
            .map_err(|e| format!("Cannot check speech environment {}: {e}", python.display()))?;
        if !result.status.success() {
            return Err(format!(
                "Invalid speech environment {}: {}",
                python.display(),
                String::from_utf8_lossy(&result.stderr).trim()
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn executable(root: &Path, revision: &str) -> PathBuf {
        let path = root.join("orion-service/target/release/orion-service");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "fixture").unwrap();
        std::fs::write(
            root.join("release.json"),
            serde_json::json!({"revision": revision}).to_string(),
        )
        .unwrap();
        path
    }

    #[test]
    fn discovers_release_at_different_depths_and_install_paths() {
        let base = tempfile::tempdir().unwrap();
        for relative in [
            "home/alice/.local/share/orion/voice-stack/releases/one",
            "opt/orion/releases/two",
            "users/Bob Smith/custom install/three",
        ] {
            let root = base.path().join(relative);
            let binary = executable(&root, relative);
            let release = InstalledRelease::from_executable(&binary).unwrap();
            assert_eq!(release.root, root.canonicalize().unwrap());
            assert_eq!(release.revision, relative);
            assert_eq!(
                release.python(),
                release.root.join("speech/.venv/bin/python")
            );
        }
    }

    #[test]
    fn missing_or_invalid_manifest_fails_instead_of_using_another_release() {
        let base = tempfile::tempdir().unwrap();
        let binary = executable(base.path(), "outer");
        std::fs::remove_file(base.path().join("release.json")).unwrap();
        assert!(
            InstalledRelease::from_executable(&binary)
                .err()
                .unwrap()
                .contains("missing release.json")
        );
        std::fs::write(base.path().join("release.json"), "not json").unwrap();
        assert!(
            InstalledRelease::from_executable(&binary)
                .err()
                .unwrap()
                .contains("Invalid Orion release metadata")
        );
        std::fs::write(base.path().join("release.json"), r#"{"revision":""}"#).unwrap();
        assert!(
            InstalledRelease::from_executable(&binary)
                .err()
                .unwrap()
                .contains("Missing revision")
        );
        std::fs::write(base.path().join("release.json"), r#"{"revision":"outer"}"#).unwrap();
        std::fs::write(
            binary.parent().unwrap().join("release.json"),
            "invalid inner manifest",
        )
        .unwrap();
        assert!(
            InstalledRelease::from_executable(&binary)
                .err()
                .unwrap()
                .contains("Invalid Orion release metadata")
        );
    }

    #[cfg(unix)]
    #[test]
    fn executable_symlink_selects_its_own_release_after_update() {
        let base = tempfile::tempdir().unwrap();
        let first = executable(&base.path().join("releases/first"), "first");
        let second = executable(&base.path().join("releases/second"), "second");
        let launcher = base.path().join("orion-service");
        std::os::unix::fs::symlink(&first, &launcher).unwrap();
        assert_eq!(
            InstalledRelease::from_executable(&launcher)
                .unwrap()
                .revision,
            "first"
        );
        std::fs::remove_file(&launcher).unwrap();
        std::os::unix::fs::symlink(&second, &launcher).unwrap();
        assert_eq!(
            InstalledRelease::from_executable(&launcher)
                .unwrap()
                .revision,
            "second"
        );
        assert_eq!(
            InstalledRelease::from_executable(&first).unwrap().revision,
            "first"
        );
    }
}
