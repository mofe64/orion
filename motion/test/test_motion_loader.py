"""Tests for Orion's YAML loading boundary."""

from pathlib import Path

import pytest
import yaml

from orion_motion.motion_loader import (
    MotionSyntaxError,
    load_yaml_file,
)


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


def test_canonical_rest_lighting_remains_a_name_after_yaml_round_trip():
    poses = load_yaml_file(Path(__file__).resolve().parents[1] / "config/v1/poses.yaml")

    assert poses["poses"]["rest"]["default_lighting"] == "off"
    reloaded = yaml.safe_load(yaml.safe_dump(poses))
    assert reloaded["poses"]["rest"]["default_lighting"] == "off"
