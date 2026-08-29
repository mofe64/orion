"""Load Orion motion data from YAML without applying semantic validation."""

from pathlib import Path
from typing import Any

import yaml


class MotionLoadError(Exception):
    """Base exception for failures while loading Orion motion data."""


class MotionFileError(MotionLoadError):
    """Raised when a motion data file cannot be read."""


class MotionSyntaxError(MotionLoadError):
    """Raised when a motion data file contains invalid or unsafe YAML."""


def load_yaml_file(path: str | Path) -> Any:
    """Read a UTF-8 YAML file and return ordinary Python data.

    This function handles only file access and YAML syntax. A successfully
    loaded value is not necessarily valid Orion motion data.
    """

    file_path = Path(path)

    try:
        contents = file_path.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as error:
        raise MotionFileError(
            f"Could not read Orion motion data file '{file_path}': {error}"
        ) from error

    try:
        return yaml.safe_load(contents)
    except yaml.YAMLError as error:
        raise MotionSyntaxError(
            f"Invalid YAML in Orion motion data file '{file_path}': {error}"
        ) from error
