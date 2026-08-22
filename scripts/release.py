#!/usr/bin/env python3
"""Prepare, verify, commit, tag, and optionally push a release."""

from __future__ import annotations

import re
import subprocess
import sys
import tomllib
from datetime import date
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
CARGO_MANIFEST = ROOT / "Cargo.toml"
PLUGIN_MANIFEST = ROOT / "herdr-plugin.toml"
LOCKFILE = ROOT / "Cargo.lock"
CHANGELOG = ROOT / "CHANGELOG.md"
RELEASE_BRANCH = "main"
UNRELEASED_HEADING = "## [Unreleased]"
VERSION_PATTERN = re.compile(r"^(\d+)\.(\d+)\.(\d+)$")


def fail(message: str) -> None:
    raise SystemExit(f"release: {message}")


def git(*args: str, capture: bool = True) -> str:
    result = subprocess.run(
        ["git", *args],
        cwd=ROOT,
        check=True,
        text=True,
        stdout=subprocess.PIPE if capture else None,
    )
    return result.stdout.strip() if result.stdout is not None else ""


def parse_version(value: str, label: str) -> tuple[int, int, int]:
    match = VERSION_PATTERN.fullmatch(value)
    if match is None:
        fail(f"{label} is not a three-part version: {value!r}.")
    return tuple(int(part) for part in match.groups())


def read_versions() -> tuple[str, str]:
    with CARGO_MANIFEST.open("rb") as source:
        cargo_version = tomllib.load(source)["package"]["version"]
    with PLUGIN_MANIFEST.open("rb") as source:
        plugin_version = tomllib.load(source)["version"]
    return cargo_version, plugin_version


def replace_version(path: Path, section: str | None, old: str, new: str) -> None:
    lines = path.read_text().splitlines(keepends=True)
    current_section: str | None = None
    replacements = 0

    for index, line in enumerate(lines):
        heading = re.fullmatch(r"\s*\[([^]]+)]\s*\n?", line)
        if heading is not None:
            current_section = heading.group(1)
            continue
        if current_section != section:
            continue
        match = re.fullmatch(r'(\s*version\s*=\s*")([^"]+)("\s*\n?)', line)
        if match is not None:
            if match.group(2) != old:
                fail(f"{path.name} changed while preparing the release.")
            lines[index] = f"{match.group(1)}{new}{match.group(3)}"
            replacements += 1

    if replacements != 1:
        fail(f"expected one version field in {path.name}, found {replacements}.")
    path.write_text("".join(lines))


def unreleased_section(changelog: str) -> str:
    start = changelog.find(UNRELEASED_HEADING)
    if start == -1:
        fail(f"CHANGELOG.md has no {UNRELEASED_HEADING} section.")
    rest = changelog[start + len(UNRELEASED_HEADING) :]
    next_heading = re.search(r"^## ", rest, flags=re.MULTILINE)
    return rest if next_heading is None else rest[: next_heading.start()]


def restore_release_files() -> None:
    git(
        "checkout",
        "--",
        CARGO_MANIFEST.name,
        PLUGIN_MANIFEST.name,
        LOCKFILE.name,
        CHANGELOG.name,
    )


def main() -> None:
    args = sys.argv[1:]
    push = "--push" in args
    requested = next((argument for argument in args if not argument.startswith("-")), None)
    unknown = [argument for argument in args if argument.startswith("-") and argument != "--push"]
    if requested is None or unknown or len(args) != 1 + int(push):
        fail("usage: just release <version> [--push]")

    requested_parts = parse_version(requested, "the requested version")
    cargo_version, plugin_version = read_versions()
    if cargo_version != plugin_version:
        fail(
            "Cargo.toml and herdr-plugin.toml versions differ: "
            f"{cargo_version!r} != {plugin_version!r}."
        )
    current_parts = parse_version(cargo_version, "the current version")
    if requested_parts < current_parts:
        fail(f"{requested} is below the current version {cargo_version}.")

    tag = f"v{requested}"
    if git("status", "--porcelain"):
        fail("the working tree has uncommitted changes. Commit or stash them first.")
    branch = git("rev-parse", "--abbrev-ref", "HEAD")
    if branch != RELEASE_BRANCH:
        fail(f"releases run from {RELEASE_BRANCH}, but {branch} is checked out.")
    if git("tag", "--list", tag):
        fail(f"{tag} already exists.")

    changelog = CHANGELOG.read_text()
    if re.search(r"^\s*-\s+\S", unreleased_section(changelog), flags=re.MULTILINE) is None:
        fail(f"{UNRELEASED_HEADING} has no entries, so there is nothing to release.")
    if re.search(rf"^## \[{re.escape(requested)}]", changelog, flags=re.MULTILINE):
        fail(f"CHANGELOG.md already contains a {requested} release.")

    replace_version(CARGO_MANIFEST, "package", cargo_version, requested)
    replace_version(PLUGIN_MANIFEST, None, plugin_version, requested)
    release_heading = f"{UNRELEASED_HEADING}\n\n## [{requested}] - {date.today().isoformat()}"
    CHANGELOG.write_text(changelog.replace(UNRELEASED_HEADING, release_heading, 1))
    print(f"release: prepared {requested}, updating Cargo.lock and running the full gate.")

    try:
        subprocess.run(["cargo", "check", "--workspace"], cwd=ROOT, check=True)
        subprocess.run(["just", "verify"], cwd=ROOT, check=True)
    except (OSError, subprocess.CalledProcessError):
        restore_release_files()
        fail("verification failed. Release metadata was restored.")

    git(
        "add",
        CARGO_MANIFEST.name,
        PLUGIN_MANIFEST.name,
        LOCKFILE.name,
        CHANGELOG.name,
    )
    git("commit", "-m", f"chore: release {requested}", capture=False)
    git("tag", tag)

    if not push:
        print(f"release: committed and tagged {tag}. Publish it with:")
        print(f"\n  git push origin {RELEASE_BRANCH} && git push origin {tag}\n")
        print(f"Undo it before pushing with:\n\n  git tag -d {tag} && git reset --hard HEAD~1\n")
        return

    git("push", "origin", RELEASE_BRANCH, capture=False)
    git("push", "origin", tag, capture=False)
    print(f"release: pushed {tag}. The release workflow publishes from here.")


if __name__ == "__main__":
    main()
