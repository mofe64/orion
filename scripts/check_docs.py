#!/usr/bin/env python3
"""Validate Orion-owned Markdown structure, local links, and current terminology."""

from __future__ import annotations

import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path
from urllib.parse import unquote


ROOT = Path(__file__).resolve().parents[1]
EXCLUDED_PARTS = {".scratch", ".venv", "node_modules", "target"}
HEADING = re.compile(r"^(#{1,6})\s+(.+?)\s*#*\s*$")
LINK = re.compile(r"(?<!!)\[[^\]]*\]\(([^)]+)\)")
EXTERNAL_PREFIXES = ("http://", "https://", "mailto:", "tel:")
OBSOLETE_MOTION_DOCUMENTATION = (
    re.compile(r"\bmotion_limits\.yaml\b", re.IGNORECASE),
    re.compile(r"\b(?:pose|motion|scene)s?\s+v1\b", re.IGNORECASE),
    re.compile(r"\bv1\s+(?:pose|motion|scene)s?\b", re.IGNORECASE),
    re.compile(r"\blegacy\s+(?:pose|motion|scene|movement)s?\b", re.IGNORECASE),
)


def markdown_files() -> list[Path]:
    commands = (
        ["git", "ls-files", "*.md"],
        ["git", "ls-files", "--others", "--exclude-standard", "*.md"],
    )
    names: set[str] = set()
    for command in commands:
        output = subprocess.check_output(command, cwd=ROOT, text=True)
        names.update(output.splitlines())

    files = []
    for name in sorted(names):
        path = ROOT / name
        if any(part in EXCLUDED_PARTS for part in path.relative_to(ROOT).parts):
            continue
        if path.is_file():
            files.append(path)
    return files


def github_slug(text: str) -> str:
    text = re.sub(r"!\[[^\]]*\]\([^)]*\)", "", text)
    text = re.sub(r"\[([^\]]+)\]\([^)]*\)", r"\1", text)
    text = text.replace("`", "").strip().lower()
    text = re.sub(r"[^\w\- ]", "", text, flags=re.UNICODE)
    text = re.sub(r"\s+", "-", text)
    return text


def anchors(path: Path) -> set[str]:
    result: set[str] = set()
    counts: defaultdict[str, int] = defaultdict(int)
    for line in path.read_text(encoding="utf-8").splitlines():
        match = HEADING.match(line)
        if not match:
            continue
        base = github_slug(match.group(2))
        occurrence = counts[base]
        counts[base] += 1
        result.add(base if occurrence == 0 else f"{base}-{occurrence}")
    return result


def main() -> int:
    files = markdown_files()
    failures: list[str] = []
    anchor_cache: dict[Path, set[str]] = {}

    for path in files:
        relative = path.relative_to(ROOT)
        lines = path.read_text(encoding="utf-8").splitlines()
        h1_lines = [number for number, line in enumerate(lines, 1) if line.startswith("# ")]
        if len(h1_lines) != 1:
            failures.append(
                f"{relative}: expected exactly one level-one heading, found {len(h1_lines)}"
            )

        for number, line in enumerate(lines, 1):
            if any(pattern.search(line) for pattern in OBSOLETE_MOTION_DOCUMENTATION):
                failures.append(
                    f"{relative}:{number}: obsolete motion-system documentation"
                )
            for raw_target in LINK.findall(line):
                target = raw_target.strip().strip("<>")
                if not target or target.startswith(EXTERNAL_PREFIXES):
                    continue

                path_part, separator, fragment = target.partition("#")
                decoded_path = unquote(path_part)
                destination = (
                    path if not decoded_path else (path.parent / decoded_path).resolve()
                )

                if not destination.exists():
                    failures.append(f"{relative}:{number}: missing link target {target}")
                    continue

                if separator and fragment and destination.suffix.lower() == ".md":
                    expected = unquote(fragment).lower()
                    known = anchor_cache.setdefault(destination, anchors(destination))
                    if expected not in known:
                        failures.append(
                            f"{relative}:{number}: missing section #{fragment} in "
                            f"{destination.relative_to(ROOT)}"
                        )

    if failures:
        print("Documentation validation failed:")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print(f"Documentation validation passed for {len(files)} Orion Markdown files.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
