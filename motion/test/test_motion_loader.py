"""Tests for Orion's YAML loading boundary."""

import pytest

from orion_motion.motion_loader import (
    MotionFileError,
    MotionSyntaxError,
    load_yaml_file,
)


def test_load_yaml_file_returns_python_data(tmp_path):
    yaml_path = tmp_path / "valid.yaml"
    yaml_path.write_text("poses:\n  home:\n    value: 0.0\n", encoding="utf-8")

    loaded = load_yaml_file(yaml_path)

    assert loaded == {"poses": {"home": {"value": 0.0}}}


def test_load_yaml_file_reports_missing_file(tmp_path):
    missing_path = tmp_path / "missing.yaml"

    with pytest.raises(MotionFileError, match="missing.yaml"):
        load_yaml_file(missing_path)


def test_load_yaml_file_reports_invalid_yaml(tmp_path):
    yaml_path = tmp_path / "invalid.yaml"
    yaml_path.write_text("poses:\n  home: [0.0\n", encoding="utf-8")

    with pytest.raises(MotionSyntaxError, match="invalid.yaml"):
        load_yaml_file(yaml_path)


def test_load_yaml_file_rejects_python_object_tags(tmp_path):
    yaml_path = tmp_path / "unsafe.yaml"
    yaml_path.write_text(
        '!!python/object/apply:os.system ["echo unsafe"]\n', encoding="utf-8"
    )

    with pytest.raises(MotionSyntaxError, match="unsafe.yaml"):
        load_yaml_file(yaml_path)


def test_load_yaml_file_leaves_empty_content_for_validator(tmp_path):
    yaml_path = tmp_path / "empty.yaml"
    yaml_path.write_text("", encoding="utf-8")

    assert load_yaml_file(yaml_path) is None
