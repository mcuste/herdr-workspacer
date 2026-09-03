#!/usr/bin/env python3
"""Write the changelog section for a release tag."""

from __future__ import annotations

import re
import sys
from pathlib import Path


CHANGELOG = Path(__file__).resolve().parents[1] / "CHANGELOG.md"
TAG_PATTERN = re.compile(r"^v(\d+\.\d+\.\d+)$")


def fail(message: str) -> None:
    raise SystemExit(f"release notes: {message}")


def main() -> None:
    if len(sys.argv) != 2:
        fail("usage: release-notes.py v<version>")

    tag = sys.argv[1]
    match = TAG_PATTERN.fullmatch(tag)
    if match is None:
        fail(f"invalid release tag {tag!r}")

    version = match.group(1)
    changelog = CHANGELOG.read_text()
    section = re.search(
        rf"^## \[{re.escape(version)}\].*?(?=^## |\Z)",
        changelog,
        flags=re.MULTILINE | re.DOTALL,
    )
    if section is None:
        fail(f"CHANGELOG.md has no section for {version}")

    print(section.group().strip())


if __name__ == "__main__":
    main()
