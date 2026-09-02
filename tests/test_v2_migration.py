from __future__ import annotations

import subprocess
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "migrate_v2_user_assets.sh"


def test_migration_archives_only_non_v2_user_assets(tmp_path: Path) -> None:
    project = tmp_path / "orion"
    archive_root = tmp_path / "backups"
    scene_directory = project / "scenes" / "user"
    pose_directory = project / "motion" / "user" / "poses"
    motion_directory = project / "motion" / "motions" / "user"
    for directory in (scene_directory, pose_directory, motion_directory):
        directory.mkdir(parents=True)

    current = scene_directory / "current.yaml"
    legacy = scene_directory / "legacy.yaml"
    malformed = pose_directory / "malformed.yml"
    current.write_text("format_version: 2\nscene: {}\n", encoding="utf-8")
    legacy.write_text("format_version: 1\nscene: {}\n", encoding="utf-8")
    malformed.write_text("scene: {}\n", encoding="utf-8")

    completed = subprocess.run(
        ["bash", str(SCRIPT), str(project), str(archive_root)],
        check=False,
        capture_output=True,
        text=True,
    )

    assert completed.returncode == 0, completed.stderr
    assert current.exists()
    assert not legacy.exists()
    assert not malformed.exists()
    archives = list(archive_root.glob("user-assets-pre-v2-*"))
    assert len(archives) == 1
    archive = archives[0]
    assert (archive / "scenes" / "user" / "legacy.yaml").exists()
    assert (archive / "motion" / "user" / "poses" / "malformed.yml").exists()
    manifest = (archive / "MANIFEST.txt").read_text(encoding="utf-8")
    assert "Orion v2 breaking-release user asset archive" in manifest
    assert "files=2" in manifest


def test_migration_refuses_ambiguous_paths(tmp_path: Path) -> None:
    completed = subprocess.run(
        ["bash", str(SCRIPT), str(tmp_path / ".." / "orion"), str(tmp_path / "backups")],
        check=False,
        capture_output=True,
        text=True,
    )
    assert completed.returncode == 2
    assert "Refusing unsafe Orion project path" in completed.stderr
