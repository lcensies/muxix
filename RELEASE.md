# Releasing

Requires [rust-release-tools](https://github.com/raine/rust-release-tools):

```bash
pipx install git+https://github.com/raine/rust-release-tools.git
```

To release:

```bash
just release --no-publish   # patch bump; add --help for other bumps
```

muxix is **not** published to crates.io (vendored path dependencies and
`[patch.crates-io]` make it unpublishable), so always pass `--no-publish`.
GitHub Releases are the only distribution channel.

This will:

1. Bump version in Cargo.toml
2. Generate changelog entry using Claude
3. Open editor to review changelog
4. Commit, tag `vX.Y.Z`, and push

Pushing the `v*` tag triggers `.github/workflows/release.yml`, which builds the
four prebuilt binaries (`muxix-{darwin,linux}-{arm64,amd64}.tar.gz` plus
`.sha256`) and publishes them as a non-draft GitHub Release using the built-in
`GITHUB_TOKEN`. A successful Release run then triggers
`.github/workflows/sandbox-image.yml`, which pushes the sandbox images to
`ghcr.io/lcensies/muxix-sandbox`.

## Backfilling changelog

To generate changelog entries for all git tags missing from CHANGELOG.md:

```bash
update-changelog
```

This uses `cc-batch` to process multiple tags in parallel.
