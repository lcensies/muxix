# Rust project checks

set positional-arguments
set shell := ["bash", "-euo", "pipefail", "-c"]

# List available commands
default:
    @just --list

# Run all checks
check: _rust-pipeline _python-pipeline docs-check

# Run check and fail if there are uncommitted changes (for CI)
check-ci: check
    #!/usr/bin/env bash
    set -euo pipefail
    if ! git diff --quiet || ! git diff --cached --quiet; then
        echo "Error: check caused uncommitted changes"
        echo "Run 'just check' locally and commit the results"
        git diff --stat
        exit 1
    fi

# Rust: format → clippy → test (sequential)
_rust-pipeline: format-rust clippy unit-tests

# Python: format → lint → typecheck (sequential)
_python-pipeline: format-python ruff-check pyright

# Format Rust and Python files
format: format-rust format-python

# Format Rust files
format-rust:
    @cargo fmt --all

# Format Python test files
format-python:
    @ruff format tests --quiet

# Auto-fix clippy warnings, then fail on any remaining
clippy:
    @cargo clippy --fix --allow-dirty --quiet -- -D clippy::all 2>&1 | { grep -v "^0 errors" || true; }

# Build the project
build:
    cargo build --all

# Build release binary via Docker (cached deps, portable)
build-docker:
    DOCKER_BUILDKIT=1 docker build -f docker/Dockerfile.build -t muxix:build .

# Install optimized binary globally from local source (fast thin-LTO profile)
install:
    cargo install --offline --path . --locked --profile release-fast

# Install fully-optimized binary from local source (fat LTO, matches CI release; slow)
install-full:
    cargo install --offline --path . --locked

# Build via Docker then install binary to host (~/.cargo/bin)
install-docker: build-docker
    #!/usr/bin/env bash
    set -euo pipefail
    install_dir="${CARGO_INSTALL_ROOT:-${CARGO_HOME:-$HOME/.cargo}}/bin"
    mkdir -p "$install_dir"
    docker run --rm --entrypoint cat muxix:build /usr/local/bin/muxix > "$install_dir/muxix"
    chmod +x "$install_dir/muxix"
    echo "Installed to $install_dir/muxix"

# Patch the local fork's muxix into the pulled sandbox image (needed because
# the upstream ghcr.io image lacks fork subcommands like `signal`/`hooks-report`).
# Uses the Docker-built bookworm-glibc binary since the host toolchain has no musl std.
sandbox-install-dev: build-docker
    #!/usr/bin/env bash
    set -euo pipefail
    # ponytail: install-dev --skip-build expects the musl target path; the
    # bookworm-glibc binary works in the bookworm image, so we park it there.
    dest=target/x86_64-unknown-linux-musl/release
    mkdir -p "$dest"
    docker run --rm --entrypoint cat muxix:build /usr/local/bin/muxix > "$dest/muxix"
    chmod +x "$dest/muxix"
    muxix sandbox install-dev --skip-build --release

# Install release binary globally from GitHub releases
install-release:
    #!/usr/bin/env bash
    set -euo pipefail
    install_root="${CARGO_INSTALL_ROOT:-${CARGO_HOME:-$HOME/.cargo}}"
    MUXIX_INSTALL_DIR="$install_root/bin" bash scripts/install.sh

# Install debug binary globally via symlink
install-dev:
    cargo build && ln -sf $(pwd)/target/debug/muxix ~/.cargo/bin/muxix

# Run unit tests
unit-tests:
    #!/usr/bin/env bash
    set -euo pipefail
    output=$(cargo test --bin muxix --quiet 2>&1) || { echo "$output"; exit 1; }
    echo "$output" | tail -1

# Run ruff linter on Python tests
ruff-check:
    @ruff check tests --fix --quiet

# Run pyright type checker on Python tests
pyright:
    #!/usr/bin/env bash
    set -euo pipefail
    source tests/venv/bin/activate
    output=$(pyright tests 2>&1) || { echo "$output"; exit 1; }
    echo "$output" | grep -v "^0 errors" || true

# Check that all docs pages have meta descriptions
docs-check:
    #!/usr/bin/env bash
    set -euo pipefail
    missing=()
    while IFS= read -r file; do
        if ! head -20 "$file" | grep -q '^description:'; then
            missing+=("$file")
        fi
    done < <(find docs -name "*.md" -not -path "*/node_modules/*" -not -path "docs/README.md")
    if [ ${#missing[@]} -gt 0 ]; then
        echo "Missing meta description in:"
        printf '  %s\n' "${missing[@]}"
        exit 1
    fi

# Run the application
run *ARGS:
    cargo run -- "$@"

# Run Python tests in parallel (depends on build)
test *ARGS: build
    #!/usr/bin/env bash
    set -euo pipefail
    source tests/venv/bin/activate
    export MUXIX_TEST=1
    quiet_flag=""
    [[ -n "${CLAUDECODE:-}" ]] && quiet_flag="-q"
    if [ $# -eq 0 ]; then
        pytest tests/ -n auto $quiet_flag
    else
        pytest $quiet_flag "$@"
    fi

# Real CRIU checkpoint/resume e2e (needs criu + rootful runtime; uses sudo).
# Auto-skips in the normal `just test` run unless MUXIX_CRIU_E2E=1 is set.
# Rootless podman cannot CRIU-checkpoint, so the test drives muxix under sudo;
# pre-cache credentials with `sudo -v` so it doesn't stall mid-run.
test-criu-e2e: build
    #!/usr/bin/env bash
    set -euo pipefail
    sudo -v
    export MUXIX_CRIU_E2E=1 MUXIX_TEST=1
    criu_run() { if command -v criu >/dev/null; then "$@"; else nix-shell -p criu --run "$*"; fi; }
    criu_run tests/venv/bin/python -m pytest tests/test_sandbox_checkpoint_e2e.py -v -s

# Run docs dev server
docs:
    cd docs && npm install && npm run dev -- --open

# Format documentation files
format-docs:
    cd docs && npm run format

# Live preview of a talk deck: base (full) or sec (security). Default base.
slides deck="base": _slides-deps
    cd presentation && npx slidev presentation-{{deck}}.md --open

# Install slide deps (slidev + playwright chromium), idempotent
_slides-deps:
    #!/usr/bin/env bash
    set -euo pipefail
    cd presentation
    [ -d node_modules ] || npm install
    npx playwright install chromium >/dev/null 2>&1 || true

# Pick a runnable Chromium: prefer system one (works on NixOS), else playwright's
_chromium := `command -v chromium || command -v chromium-browser || command -v google-chrome-stable || true`

# Export a deck (base|sec) to dist/<deck>.pdf (via playwright)
slides-pdf deck="base": _slides-deps
    #!/usr/bin/env bash
    set -euo pipefail
    cd presentation
    exe="{{_chromium}}"
    if [ -n "$exe" ]; then
        npx slidev export presentation-{{deck}}.md --output dist/{{deck}}.pdf --executable-path "$exe"
    else
        npx slidev export presentation-{{deck}}.md --output dist/{{deck}}.pdf
    fi
    echo "→ presentation/dist/{{deck}}.pdf"

# Export a deck (base|sec) to per-page PNGs in dist/png-<deck>/
slides-png deck="base": _slides-deps
    #!/usr/bin/env bash
    set -euo pipefail
    cd presentation
    exe="{{_chromium}}"
    if [ -n "$exe" ]; then
        npx slidev export presentation-{{deck}}.md --format png --output dist/png-{{deck}} --executable-path "$exe"
    else
        npx slidev export presentation-{{deck}}.md --format png --output dist/png-{{deck}}
    fi
    echo "→ presentation/dist/png-{{deck}}/"

# Build a deck (base|sec) static site into presentation/dist/<deck>/
slides-build deck="base": _slides-deps
    cd presentation && npx slidev build presentation-{{deck}}.md --out dist/{{deck}}
    @echo "→ presentation/dist/{{deck}}/"

# Release a new patch version
release *ARGS:
    @just _release patch {{ARGS}}

# Internal release helper
_release bump *ARGS:
    @cargo-release {{bump}} {{ARGS}}
