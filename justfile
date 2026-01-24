# Rust project checks

set positional-arguments
set shell := ["bash", "-euo", "pipefail", "-c"]

# List available commands
default:
    @just --list

# Run format, clippy-fix, build, and unit tests
check: parallel-checks clippy

# Run format, clippy-fix, build, unit tests, ruff, pyright, and docs checks in parallel
[parallel]
parallel-checks: format clippy-fix build unit-tests ruff-check pyright docs-check

# Format Rust and Python files
format:
    cargo fmt --all
    ruff format tests --quiet

# Run clippy and fail on any warnings
clippy:
    cargo clippy -- -D clippy::all

# Auto-fix clippy warnings
clippy-fix:
    cargo clippy --fix --allow-dirty -- -W clippy::all

# Build the project
build:
    cargo build --all

# Install debug binary globally via symlink
install-dev:
    cargo build && ln -sf $(pwd)/target/debug/workmux ~/.cargo/bin/workmux

# Run unit tests
unit-tests:
    cargo test --bin workmux --quiet

# Run ruff linter on Python tests
ruff-check:
    ruff check tests --fix

# Run pyright type checker on Python tests
pyright:
    #!/usr/bin/env bash
    set -euo pipefail
    source tests/venv/bin/activate
    pyright tests

# Check that all docs pages have meta descriptions
docs-check:
    #!/usr/bin/env bash
    set -euo pipefail
    missing=()
    while IFS= read -r file; do
        if ! head -20 "$file" | grep -q '^description:'; then
            missing+=("$file")
        fi
    done < <(find docs -name "*.md" -not -path "*/node_modules/*")
    if [ ${#missing[@]} -gt 0 ]; then
        echo "Missing meta description in:"
        printf '  %s\n' "${missing[@]}"
        exit 1
    fi
    echo "All docs have descriptions"

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
    cd docs && npm install && npm run format

# Release a new patch version
release *ARGS:
    @just _release patch {{ARGS}}

# Internal release helper
_release bump *ARGS:
    @cargo-release {{bump}} {{ARGS}}
