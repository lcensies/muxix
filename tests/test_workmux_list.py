import os
import re
from pathlib import Path
from typing import Dict, List

from .conftest import (
    MuxEnvironment,
    create_commit,
    get_window_name,
    get_worktree_path,
    run_workmux_add,
    run_workmux_command,
    write_workmux_config,
)


def run_workmux_list(
    env: MuxEnvironment, workmux_exe_path: Path, repo_path: Path
) -> str:
    """
    Runs `workmux list` inside the multiplexer session and returns the output.
    """
    run_workmux_command(env, workmux_exe_path, repo_path, "list")
    stdout_file = env.tmp_path / "workmux_stdout.txt"
    return stdout_file.read_text()


def parse_list_output(output: str) -> List[Dict[str, str]]:
    """
    Parses the tabular output of `workmux list` into a list of dictionaries.
    This parser is robust to variable column widths.
    """
    lines = [line.rstrip() for line in output.strip().split("\n")]
    if len(lines) < 1:  # Header at minimum
        return []

    header = lines[0]
    # Use regex to find column headers, robust against extra spaces
    columns = re.split(r"\s{2,}", header.strip())
    columns = [c.strip() for c in columns if c.strip()]

    # Find the start index of each column in the header string
    indices = [header.find(col) for col in columns]

    results = []
    # Data rows start after the header (no separator line in blank style)
    for row_str in lines[1:]:
        if not row_str.strip():  # Skip empty lines
            continue
        row_data = {}
        for i, col_name in enumerate(columns):
            start = indices[i]
            # The last column goes to the end of the line
            end = indices[i + 1] if i + 1 < len(indices) else len(row_str)
            value = row_str[start:end].strip()
            row_data[col_name] = value
        results.append(row_data)

    return results


def test_list_output_format(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Verifies `workmux list` produces correctly formatted table output."""
    env = mux_server
    branch_name = "feature-test"
    write_workmux_config(mux_repo_path)
    run_workmux_add(env, workmux_exe_path, mux_repo_path, branch_name)

    output = run_workmux_list(env, workmux_exe_path, mux_repo_path)
    worktree_path = get_worktree_path(mux_repo_path, branch_name)

    # Parse and verify the output contains the expected data
    parsed_output = parse_list_output(output)
    assert len(parsed_output) == 2

    # Verify header is present
    assert "BRANCH" in output
    assert "MUX" in output
    assert "UNMERGED" in output
    assert "PATH" in output

    # Verify main branch entry - should show "(here)" when run from mux_repo_path
    main_entry = next((r for r in parsed_output if r["BRANCH"] == "main"), None)
    assert main_entry is not None
    assert main_entry["MUX"] == "-"
    assert main_entry["UNMERGED"] == "-"
    assert main_entry["PATH"] == "(here)"

    # Verify feature branch entry - shows as relative path
    feature_entry = next((r for r in parsed_output if r["BRANCH"] == branch_name), None)
    assert feature_entry is not None
    assert feature_entry["MUX"] == "✓"
    assert feature_entry["UNMERGED"] == "-"
    # Convert relative path to absolute and compare
    expected_relative = os.path.relpath(worktree_path, mux_repo_path)
    assert feature_entry["PATH"] == expected_relative


def test_list_initial_state(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Verifies `workmux list` shows only the main branch in a new repo."""
    env = mux_server

    output = run_workmux_list(env, workmux_exe_path, mux_repo_path)
    parsed_output = parse_list_output(output)
    assert len(parsed_output) == 1

    main_entry = parsed_output[0]
    assert main_entry["BRANCH"] == "main"
    assert main_entry["MUX"] == "-"
    assert main_entry["UNMERGED"] == "-"
    # When run from mux_repo_path, main branch shows as "(here)"
    assert main_entry["PATH"] == "(here)"


def test_list_with_active_worktree(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Verifies `list` shows an active worktree with a multiplexer window ('✓')."""
    env = mux_server
    branch_name = "feature-active"
    write_workmux_config(mux_repo_path)

    # Create the worktree and window
    run_workmux_add(env, workmux_exe_path, mux_repo_path, branch_name)

    output = run_workmux_list(env, workmux_exe_path, mux_repo_path)
    parsed_output = parse_list_output(output)
    assert len(parsed_output) == 2

    worktree_entry = next(
        (r for r in parsed_output if r["BRANCH"] == branch_name), None
    )
    assert worktree_entry is not None
    assert worktree_entry["MUX"] == "✓"
    assert worktree_entry["UNMERGED"] == "-"
    # Path shows as relative when run from mux_repo_path
    expected_path = get_worktree_path(mux_repo_path, branch_name)
    expected_relative = os.path.relpath(expected_path, mux_repo_path)
    assert worktree_entry["PATH"] == expected_relative


def test_list_with_unmerged_commits(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Verifies `list` shows a worktree with unmerged commits ('●')."""
    env = mux_server
    branch_name = "feature-unmerged"
    worktree_path = get_worktree_path(mux_repo_path, branch_name)
    write_workmux_config(mux_repo_path)
    run_workmux_add(env, workmux_exe_path, mux_repo_path, branch_name)

    # Create a commit only on the feature branch
    create_commit(env, worktree_path, "This commit is unmerged")

    output = run_workmux_list(env, workmux_exe_path, mux_repo_path)
    parsed_output = parse_list_output(output)
    worktree_entry = next(
        (r for r in parsed_output if r["BRANCH"] == branch_name), None
    )
    assert worktree_entry is not None
    assert worktree_entry["MUX"] == "✓"
    assert worktree_entry["UNMERGED"] == "●"


def test_list_with_detached_window(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Verifies `list` shows a worktree whose window has been closed ('-')."""
    env = mux_server
    branch_name = "feature-detached"
    window_name = get_window_name(branch_name)
    write_workmux_config(mux_repo_path)
    run_workmux_add(env, workmux_exe_path, mux_repo_path, branch_name)

    # Kill the window manually
    env.kill_window(window_name)

    output = run_workmux_list(env, workmux_exe_path, mux_repo_path)
    parsed_output = parse_list_output(output)
    worktree_entry = next(
        (r for r in parsed_output if r["BRANCH"] == branch_name), None
    )
    assert worktree_entry is not None
    assert worktree_entry["MUX"] == "-"
    assert worktree_entry["UNMERGED"] == "-"


def test_list_alias_ls_works(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Verifies that the `ls` alias for `list` works correctly."""
    env = mux_server

    # Run `ls` and verify it produces expected output
    run_workmux_command(env, workmux_exe_path, mux_repo_path, "ls")
    stdout_file = env.tmp_path / "workmux_stdout.txt"
    ls_output = stdout_file.read_text()

    parsed_output = parse_list_output(ls_output)
    assert len(parsed_output) == 1
    assert parsed_output[0]["BRANCH"] == "main"
    # When run from mux_repo_path, main branch shows as "(here)"
    assert parsed_output[0]["PATH"] == "(here)"
