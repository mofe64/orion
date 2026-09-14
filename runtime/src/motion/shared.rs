use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{OrionRuntimeError, Result};

/// Collects all YAML files in the given directory and its subdirectories.
pub(super) fn collect_yaml_files(
    directory: &Path,
    files: &mut Vec<PathBuf>,
    library_label: &str,
) -> Result<()> {
    for entry in fs::read_dir(directory).map_err(|error| {
        OrionRuntimeError::Runtime(format!(
            "Could not read {library_label} '{}': {error}",
            directory.display()
        ))
    })? {
        let path = entry?.path();
        if path.is_dir() {
            collect_yaml_files(&path, files, library_label)?;
        } else if matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("yaml" | "yml")
        ) {
            files.push(path);
        }
    }
    Ok(())
}

/// Returns `true` if the given string is a semantic name (i.e. it contains only ASCII alphanumeric characters or underscores/dashes).
pub(super) fn is_semantic_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

#[cfg(test)]
pub(super) fn fixture_path(name: &str, subdir: &str) -> PathBuf {
    let fixtures_dir = format!("tests/fixtures/{}", subdir);
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(fixtures_dir)
        .join(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn collect_yaml_files_recurses_filters_extensions_and_appends() {
        let root = tempfile::tempdir().unwrap();
        let nested = root.path().join("nested/deeper");
        fs::create_dir_all(&nested).unwrap();
        for relative in [
            "first.yaml",
            "nested/second.yml",
            "nested/deeper/third.yaml",
            "ignored.json",
            "ignored.YAML",
            "no_extension",
        ] {
            fs::copy(
                fixture_path("studio_keyframe.yaml", "poses"),
                root.path().join(relative),
            )
            .unwrap();
        }
        let existing = fixture_path("metadata.yaml", "poses");
        let mut files = vec![existing.clone()];
        collect_yaml_files(root.path(), &mut files, "user pose library").unwrap();
        // Directory enumeration order is not guaranteed; compare sets.
        assert_eq!(files.len(), 4);
        assert_eq!(
            files.into_iter().collect::<BTreeSet<_>>(),
            [
                existing,
                root.path().join("first.yaml"),
                root.path().join("nested/second.yml"),
                nested.join("third.yaml")
            ]
            .into_iter()
            .collect()
        );
    }

    #[test]
    fn collect_yaml_files_accepts_empty_directory() {
        let root = tempfile::tempdir().unwrap();
        let mut files = Vec::new();
        collect_yaml_files(root.path(), &mut files, "user pose library").unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn collect_yaml_files_reports_missing_directory_and_regular_file() {
        let root = tempfile::tempdir().unwrap();
        for path in [
            root.path().join("missing"),
            fixture_path("metadata.yaml", "poses"),
        ] {
            let error =
                collect_yaml_files(&path, &mut Vec::new(), "user pose library").unwrap_err();
            assert!(matches!(error, OrionRuntimeError::Runtime(_)));
            let message = error.to_string();
            assert!(message.contains("Could not read user pose library"));
            assert!(message.contains(&path.display().to_string()));
        }
    }

    #[test]
    fn semantic_names_accept_ascii_letters_digits_underscores_and_hyphens() {
        for name in ["home", "Home2", "pose_name-1", "0", "_", "-"] {
            assert!(is_semantic_name(name), "rejected {name:?}");
        }
        for name in [
            "",
            " ",
            "home pose",
            "home.pose",
            "home/pose",
            "pose!",
            "café",
            "姿勢",
            "home\n",
            "\t",
        ] {
            assert!(!is_semantic_name(name), "accepted {name:?}");
        }
    }
}
