# Rust project checks

set positional-arguments
set shell := ["bash", "-euo", "pipefail", "-c"]

# List available commands
default:
    @just --list

# Run all checks via three parallel pipelines
[parallel]
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

# Install release binary globally
install:
    cargo install --offline --path . --locked

# Install debug binary globally via symlink
install-dev:
    cargo build && ln -sf $(pwd)/target/debug/workmux ~/.cargo/bin/workmux

# Run unit tests
unit-tests:
    #!/usr/bin/env bash
    set -euo pipefail
    output=$(cargo test --bin workmux --quiet 2>&1) || { echo "$output"; exit 1; }
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
    export WORKMUX_TEST=1
    quiet_flag=""
    [[ -n "${CLAUDECODE:-}" ]] && quiet_flag="-q"
    if [ $# -eq 0 ]; then
        pytest tests/ -n auto $quiet_flag
    else
        pytest $quiet_flag "$@"
    fi

# Run docs dev server
docs:
    cd docs && npm install && npm run dev -- --open

# Format documentation files
format-docs:
    cd docs && npm run format

# Release a new patch version
release *ARGS:
    @just _release patch {{ARGS}}

# Internal release helper
_release bump *ARGS:
    @cargo-release {{bump}} {{ARGS}}
