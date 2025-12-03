#!/usr/bin/env python3

import argparse
import os
import pathlib
import re
import subprocess
import sys
from typing import Tuple

import update_changelog

ROOT = pathlib.Path(__file__).resolve().parent.parent
CARGO_TOML = ROOT / "Cargo.toml"


def run(cmd: list[str], *, check: bool = True) -> subprocess.CompletedProcess:
    """Execute a command relative to the repo root."""
    return subprocess.run(cmd, cwd=ROOT, check=check)


def run_capture(cmd: list[str]) -> str:
    result = subprocess.run(
        cmd,
        cwd=ROOT,
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    return result.stdout


def ensure_clean_worktree() -> None:
    status = run_capture(["git", "status", "--porcelain", "--ignore-submodules"])
    if status.strip():
        sys.stderr.write("error: working tree must be clean before releasing\n")
        sys.stderr.write(status)
        sys.exit(1)


def read_package_info() -> Tuple[str, str]:
    toml_text = CARGO_TOML.read_text()

    name_match = re.search(r'(?m)^\s*name\s*=\s*"([^"]+)"\s*$', toml_text)
    version_match = re.search(r'(?m)^\s*version\s*=\s*"([^"]+)"\s*$', toml_text)

    if not name_match or not version_match:
        sys.stderr.write(
            "error: unable to extract package name/version from Cargo.toml\n"
        )
        sys.exit(1)

    return name_match.group(1), version_match.group(1)


def bump_version(current: str, bump: str) -> str:
    parts = current.split(".")
    if len(parts) != 3 or any(not p.isdigit() for p in parts):
        sys.stderr.write(f"error: unsupported version format '{current}'\n")
        sys.exit(1)

    major, minor, patch = map(int, parts)

    if bump == "patch":
        patch += 1
    elif bump == "minor":
        minor += 1
        patch = 0
    elif bump == "major":
        major += 1
        minor = 0
        patch = 0
    else:
        sys.stderr.write(f"error: unknown bump '{bump}'\n")
        sys.exit(1)

    return f"{major}.{minor}.{patch}"


def update_cargo_files(new_version: str) -> None:
    toml_text = CARGO_TOML.read_text()
    new_toml_text, replaced = re.subn(
        r'(?m)^(version\s*=\s*")([^"]+)(")',
        lambda m: f"{m.group(1)}{new_version}{m.group(3)}",
        toml_text,
        count=1,
    )
    if replaced != 1:
        sys.stderr.write("error: failed to update Cargo.toml\n")
        sys.exit(1)
    CARGO_TOML.write_text(new_toml_text)

    # Let cargo update Cargo.lock automatically
    run(["cargo", "check", "--quiet"])


def commit_release(new_version: str) -> None:
    message = f"release v{new_version}"
    run(["git", "add", "Cargo.toml", "Cargo.lock", "CHANGELOG.md"])
    run(["git", "commit", "-m", message])


def tag_release(new_version: str) -> None:
    message = f"release v{new_version}"
    run(["git", "tag", "-a", f"v{new_version}", "-m", message])


def push_release() -> None:
    run(["git", "push"])
    run(["git", "push", "--tags"])


def publish_crate() -> None:
    run(["cargo", "publish"])


def main() -> None:
    # Change to repo root so update_changelog module works correctly
    os.chdir(ROOT)

    parser = argparse.ArgumentParser(description="Release helper for cargo crate")
    parser.add_argument(
        "bump",
        choices=("patch", "minor", "major"),
        help="Semver component to bump",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Generate changelog only, don't commit/publish/push",
    )
    args = parser.parse_args()

    ensure_clean_worktree()

    crate_name, current_version = read_package_info()
    new_version = bump_version(current_version, args.bump)

    update_cargo_files(new_version)

    # Generate changelog entry for the new version
    update_changelog.generate_for_pending(f"v{new_version}")

    status_after_update = run_capture(
        ["git", "status", "--porcelain", "--ignore-submodules"]
    )
    if not status_after_update.strip():
        sys.stderr.write("error: version bump produced no changes; aborting\n")
        sys.exit(1)

    if args.dry_run:
        print(f"Dry run complete for {crate_name} v{new_version}")
        print("Changes staged but not committed. Run 'git diff' to review.")
        return

    # Let user review/edit changelog before proceeding
    editor = os.environ.get("EDITOR", "vim")
    subprocess.run([editor, "CHANGELOG.md"], check=True)

    response = input("Proceed with release? [y/N] ").strip().lower()
    if response != "y":
        print("Aborting release.")
        sys.exit(1)

    commit_release(new_version)

    publish_crate()

    tag_release(new_version)
    push_release()

    print(f"Released {crate_name} v{new_version}")


if __name__ == "__main__":
    main()
