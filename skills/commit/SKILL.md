---
name: commit
description: Commit staged changes with a consistent style.
disable-model-invocation: true
allowed-tools: Read, Bash, Glob, Grep
---

<!-- Customize the commit style to match your team's conventions. -->

Commit the changes using this style:

- lowercase
- imperative mood
- concise, no conventional commit prefixes
- optionally use a context prefix when it adds clarity (e.g., "docs:", "cli:")

If nothing is staged, stage all changes first.
