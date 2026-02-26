---
description: Update workmux to the latest version
---

# update

Updates workmux to the latest version by downloading the prebuilt binary from GitHub Releases.

```bash
workmux update
```

## What happens

1. Checks the latest release version from the GitHub API
2. Compares with the currently installed version
3. Downloads the correct binary for your OS and architecture
4. Verifies the SHA-256 checksum
5. Atomically replaces the current binary (with rollback on failure)

If workmux is already up to date, it reports this and exits.

## Homebrew installs

If workmux was installed via Homebrew, the command will detect this and instruct you to use `brew upgrade` instead:

```bash
brew upgrade workmux
```

## Supported platforms

The command downloads prebuilt binaries for:

- macOS (Apple Silicon / Intel)
- Linux (x86_64 / ARM64)

## Requirements

- `curl` must be available in PATH (used for downloading)
- `tar` must be available in PATH (used for extraction)
- `sha256sum` or `shasum` must be available (used for checksum verification)
- Write permission to the directory containing the workmux binary
