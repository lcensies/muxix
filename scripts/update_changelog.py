#!/usr/bin/env python3
"""
Find git tags missing from CHANGELOG.md and use cc-batch to add them.
"""

import subprocess
import re
import tempfile
from pathlib import Path

PROMPT = """\
# Changelog Updater

## Task

Generate a user-focused changelog entry for a git tag and insert it into CHANGELOG.md.

## Input

- Git tag: `{input}`

## Steps

1. **Find the previous tag**

   ```bash
   git describe --tags --abbrev=0 '{input}^'
   ```

   If this fails (no previous tag exists), this is the first release—compare against the initial commit instead, or skip if there's nothing meaningful to document.

2. **Get the commits between tags**

   ```bash
   git log --oneline PREVIOUS_TAG..{input}
   ```

3. **Review the actual changes** to understand context

   ```bash
   git diff PREVIOUS_TAG..{input} --stat
   ```

   Read relevant changed files as needed to understand what the changes do.

4. **Get the tag date**

   ```bash
   git log -1 --format=%as {input}
   ```

5. **Update CHANGELOG.md**

   - If CHANGELOG.md doesn't exist, create it with `# Changelog` header
   - Insert the new entry in the correct position by version order (newest first)
   - Use heading level `##` for version entries

## Writing Guidelines

- Write from the user's perspective: what can they now do, what's fixed, what's improved?
- Use plain language, avoid implementation details like function names, file paths, or internal refactors
- Focus on behavior changes, new capabilities, and bug fixes users would notice
- Skip purely internal changes (refactoring, test improvements, CI changes) unless they affect users
- Keep entries concise—one line per change is usually enough
- If a tag has no user-facing changes, skip it entirely—don't modify CHANGELOG.md

## Entry Format

```markdown
## {input} (YYYY-MM-DD)

- Change description
- Another change description
```
"""

PENDING_PROMPT = """\
# Changelog Updater (Pending Release)

## Task

Generate a user-focused changelog entry for the upcoming release `{input}` and insert it into CHANGELOG.md.

## Steps

1. **Find the previous tag**

   ```bash
   git describe --tags --abbrev=0
   ```

   If this fails (no previous tag exists), this is the first release—compare against the initial commit instead, or skip if there's nothing meaningful to document.

2. **Get the commits since the last tag**

   ```bash
   git log --oneline PREVIOUS_TAG..HEAD
   ```

3. **Review the actual changes** to understand context

   ```bash
   git diff PREVIOUS_TAG..HEAD --stat
   ```

   Read relevant changed files as needed to understand what the changes do.

4. **Update CHANGELOG.md**

   - If CHANGELOG.md doesn't exist, create it with `# Changelog` header
   - Insert the new entry at the top (after the header), as the newest release
   - Use heading level `##` for version entries
   - Use today's date in YYYY-MM-DD format

## Writing Guidelines

- Write from the user's perspective: what can they now do, what's fixed, what's improved?
- Use plain language, avoid implementation details like function names, file paths, or internal refactors
- Focus on behavior changes, new capabilities, and bug fixes users would notice
- Skip purely internal changes (refactoring, test improvements, CI changes) unless they affect users
- Keep entries concise—one line per change is usually enough
- If there are no user-facing changes, still create a minimal entry noting the release

## Entry Format

```markdown
## {input} (YYYY-MM-DD)

- Change description
- Another change description
```
"""


def run_cc_batch(prompt_template: str, input_str: str) -> None:
    """Run cc-batch with a specific prompt template."""
    with tempfile.NamedTemporaryFile(
        mode="w", suffix=".md", delete=False
    ) as prompt_file:
        prompt_file.write(prompt_template)
        prompt_path = prompt_file.name

    try:
        subprocess.run(
            ["cc-batch", prompt_path, "--dangerously-skip-permissions"],
            input=input_str,
            text=True,
            check=True,
        )
    finally:
        Path(prompt_path).unlink(missing_ok=True)


def generate_for_pending(version_tag: str) -> None:
    """
    Generate a changelog entry for a pending release (HEAD) and update CHANGELOG.md.
    This is used during the release process before the tag is created.
    """
    print(f"Generating changelog entry for {version_tag}...")
    prompt = PENDING_PROMPT.replace("{input}", version_tag)
    subprocess.run(
        ["claude", "--dangerously-skip-permissions", "-p", prompt],
        check=True,
    )

    # Format with prettier
    subprocess.run(["prettier", "--write", "CHANGELOG.md"], capture_output=True)


def get_all_tags() -> list[str]:
    """Get all git tags sorted by version (newest first)."""
    result = subprocess.run(
        ["git", "tag", "--sort=-version:refname"],
        capture_output=True,
        text=True,
        check=True,
    )
    return [t.strip() for t in result.stdout.strip().split("\n") if t.strip()]


def get_tag_date(tag: str) -> str:
    """Get the date of a tag in YYYY-MM-DD format."""
    result = subprocess.run(
        ["git", "log", "-1", "--format=%as", tag],
        capture_output=True,
        text=True,
        check=True,
    )
    return result.stdout.strip()


def promote_unreleased_section(tag: str) -> bool:
    """
    Check for '## Unreleased' in CHANGELOG.md and rename it to the given tag.
    Returns True if the section was found and updated.
    """
    changelog_path = Path("CHANGELOG.md")
    if not changelog_path.exists():
        return False

    content = changelog_path.read_text()

    # Look for "## Unreleased" (flexible spacing)
    pattern = r"^(##\s+Unreleased)\s*$"
    match = re.search(pattern, content, re.MULTILINE)

    if match:
        try:
            date = get_tag_date(tag)
        except subprocess.CalledProcessError:
            print(f"Warning: Could not get date for tag {tag}")
            return False

        new_header = f"## {tag} ({date})"
        new_content = content[: match.start(1)] + new_header + content[match.end(1) :]

        changelog_path.write_text(new_content)
        return True

    return False


def get_tags_in_changelog() -> set[str]:
    """Get tags already present in CHANGELOG.md (entries or skipped markers)."""
    try:
        with open("CHANGELOG.md") as f:
            content = f.read()
        tags = set()
        # Match ## v0.1.5 or # v0.1.5 style headings (with optional date)
        heading_pattern = r"^#+\s+(v[\d.]+)"
        tags.update(re.findall(heading_pattern, content, re.MULTILINE))
        # Match <!-- skipped: v0.1.5 --> comments
        skipped_pattern = r"<!--\s*skipped:\s*(v[\d.]+)\s*-->"
        tags.update(re.findall(skipped_pattern, content))
        return tags
    except FileNotFoundError:
        return set()


def insert_skipped_comments(tags: list[str]) -> None:
    """Insert <!-- skipped: vX.X.X --> comments for tags that weren't added."""
    if not tags:
        return
    try:
        with open("CHANGELOG.md") as f:
            content = f.read()
    except FileNotFoundError:
        return

    # Insert skipped comments at the top, after the # Changelog header
    skipped_block = "\n".join(f"<!-- skipped: {tag} -->" for tag in tags)
    # Find the end of the first line (the header)
    header_end = content.find("\n")
    if header_end == -1:
        content = content + "\n\n" + skipped_block
    else:
        content = content[: header_end + 1] + "\n" + skipped_block + content[header_end + 1 :]

    with open("CHANGELOG.md", "w") as f:
        f.write(content)


def main():
    all_tags = get_all_tags()
    existing_tags = get_tags_in_changelog()

    missing_tags = [t for t in all_tags if t not in existing_tags]

    if not missing_tags:
        print("All tags are already in CHANGELOG.md")
        return

    print(f"Found {len(missing_tags)} missing tags: {', '.join(missing_tags)}")

    # Try to promote Unreleased section for the newest tag
    if missing_tags and promote_unreleased_section(missing_tags[0]):
        print(f"Promoted '## Unreleased' section to {missing_tags[0]}")
        missing_tags.pop(0)

    if not missing_tags:
        # Format with prettier
        subprocess.run(["prettier", "--write", "CHANGELOG.md"], capture_output=True)
        return

    # Run cc-batch for missing tags
    tags_input = "\n".join(missing_tags)
    run_cc_batch(PROMPT, tags_input)

    # Check which tags are still missing and mark them as skipped
    updated_tags = get_tags_in_changelog()
    still_missing = [t for t in missing_tags if t not in updated_tags]
    if still_missing:
        print(f"Marking {len(still_missing)} tags as skipped: {', '.join(still_missing)}")
        insert_skipped_comments(still_missing)

    # Format with prettier
    subprocess.run(["prettier", "--write", "CHANGELOG.md"], capture_output=True)


if __name__ == "__main__":
    main()
