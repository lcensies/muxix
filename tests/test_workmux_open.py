from pathlib import Path

from .conftest import (
    DEFAULT_WINDOW_PREFIX,
    TmuxEnvironment,
    get_window_name,
    get_worktree_path,
    poll_until,
    run_workmux_add,
    run_workmux_open,
    run_workmux_remove,
    write_workmux_config,
)


def _kill_window(env: TmuxEnvironment, branch_name: str) -> None:
    """Helper to close the tmux window for a branch if it exists."""
    window_name = get_window_name(branch_name)
    env.tmux(["has-session", "-t", window_name], check=False)
    env.tmux(["kill-window", "-t", window_name], check=False)


def _get_all_windows(env: TmuxEnvironment) -> list[str]:
    """Helper to get all tmux window names."""
    return env.tmux(["list-windows", "-F", "#{window_name}"]).stdout.splitlines()


def test_open_recreates_tmux_window_for_existing_worktree(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux open` recreates a tmux window for an existing worktree."""
    env = isolated_tmux_server
    branch_name = "feature-open-success"
    window_name = get_window_name(branch_name)

    write_workmux_config(repo_path)
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    # Close the original window to simulate a detached worktree
    env.tmux(["kill-window", "-t", window_name])

    run_workmux_open(env, workmux_exe_path, repo_path, branch_name)

    list_windows = _get_all_windows(env)
    assert window_name in list_windows


def test_open_switches_to_existing_window_by_default(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux open` switches to existing window instead of erroring."""
    env = isolated_tmux_server
    branch_name = "feature-switch-test"
    target_window = get_window_name(branch_name)

    write_workmux_config(repo_path)
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    # Open again - should switch, not error
    result = run_workmux_open(env, workmux_exe_path, repo_path, branch_name)

    assert "Switched to existing tmux window" in result.stdout

    # Should still only have one window for this worktree
    list_windows = _get_all_windows(env)
    matching = [
        w for w in list_windows if w.startswith(f"{DEFAULT_WINDOW_PREFIX}{branch_name}")
    ]
    assert len(matching) == 1

    # Verify tmux actually switched focus to the target window
    active_window = env.tmux(["display-message", "-p", "#{window_name}"]).stdout.strip()
    assert active_window == target_window, (
        f"Expected active window to be '{target_window}', got '{active_window}'"
    )


def test_open_with_new_flag_creates_duplicate_window(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux open --new` creates a duplicate window with suffix."""
    env = isolated_tmux_server
    branch_name = "feature-duplicate"
    base_window = get_window_name(branch_name)

    write_workmux_config(repo_path)
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    # Open with --new flag
    result = run_workmux_open(
        env, workmux_exe_path, repo_path, branch_name, new_window=True
    )

    assert "Opened tmux window" in result.stdout

    # Should now have two windows: base and -2
    list_windows = _get_all_windows(env)
    assert base_window in list_windows
    assert f"{base_window}-2" in list_windows


def test_open_new_without_name_uses_current_worktree(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux open --new` without name uses current worktree from cwd."""
    env = isolated_tmux_server
    branch_name = "feature-open-current"
    base_window = get_window_name(branch_name)

    write_workmux_config(repo_path)
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    # Get the worktree path to run the command from
    worktree_path = get_worktree_path(repo_path, branch_name)

    # Open with --new flag but no name, from inside the worktree directory
    result = run_workmux_open(
        env,
        workmux_exe_path,
        repo_path,
        None,
        new_window=True,
        working_dir=worktree_path,
    )

    assert "Opened tmux window" in result.stdout

    # Should now have two windows: base and -2
    list_windows = _get_all_windows(env)
    assert base_window in list_windows
    assert f"{base_window}-2" in list_windows


def test_open_with_new_flag_creates_incrementing_suffixes(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies multiple `workmux open --new` creates incrementing suffixes (-2, -3, -4)."""
    env = isolated_tmux_server
    branch_name = "feature-multi-dup"
    base_window = get_window_name(branch_name)

    write_workmux_config(repo_path)
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    # Open three more duplicates
    run_workmux_open(env, workmux_exe_path, repo_path, branch_name, new_window=True)
    run_workmux_open(env, workmux_exe_path, repo_path, branch_name, new_window=True)
    run_workmux_open(env, workmux_exe_path, repo_path, branch_name, new_window=True)

    list_windows = _get_all_windows(env)
    assert base_window in list_windows
    assert f"{base_window}-2" in list_windows
    assert f"{base_window}-3" in list_windows
    assert f"{base_window}-4" in list_windows


def test_open_new_flag_when_no_window_exists_uses_base_name(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux open --new` uses base name when no window exists."""
    env = isolated_tmux_server
    branch_name = "feature-new-no-existing"
    window_name = get_window_name(branch_name)

    write_workmux_config(repo_path)
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    # Kill the original window
    env.tmux(["kill-window", "-t", window_name])

    # Open with --new flag - should use base name since none exists
    run_workmux_open(env, workmux_exe_path, repo_path, branch_name, new_window=True)

    list_windows = _get_all_windows(env)
    assert window_name in list_windows
    # Should NOT have -2 suffix since there was no existing window
    assert f"{window_name}-2" not in list_windows


def test_open_new_flag_with_gap_appends_after_highest(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux open --new` appends after highest suffix, not filling gaps."""
    env = isolated_tmux_server
    branch_name = "feature-gap-test"
    base_window = get_window_name(branch_name)

    write_workmux_config(repo_path)
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    # Create -2 and -3 windows
    run_workmux_open(env, workmux_exe_path, repo_path, branch_name, new_window=True)
    run_workmux_open(env, workmux_exe_path, repo_path, branch_name, new_window=True)

    # Verify we have base, -2, and -3
    list_windows = _get_all_windows(env)
    assert base_window in list_windows
    assert f"{base_window}-2" in list_windows
    assert f"{base_window}-3" in list_windows

    # Kill -2 to create a gap
    env.tmux(["kill-window", "-t", f"={base_window}-2"])

    # Verify gap exists
    list_windows = _get_all_windows(env)
    assert f"{base_window}-2" not in list_windows
    assert f"{base_window}-3" in list_windows

    # Open with --new again - should create -4, not fill the -2 gap
    run_workmux_open(env, workmux_exe_path, repo_path, branch_name, new_window=True)

    list_windows = _get_all_windows(env)
    assert f"{base_window}-4" in list_windows, (
        "Should append after highest suffix (-3), creating -4"
    )
    # Gap should still exist (we don't fill gaps)
    assert f"{base_window}-2" not in list_windows, "Gap at -2 should not be filled"


def test_open_fails_when_worktree_missing(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux open` fails if the worktree does not exist."""
    env = isolated_tmux_server
    worktree_name = "missing-worktree"

    write_workmux_config(repo_path)

    result = run_workmux_open(
        env,
        workmux_exe_path,
        repo_path,
        worktree_name,
        expect_fail=True,
    )

    assert "No worktree found with name" in result.stderr


def test_open_with_run_hooks_reexecutes_post_create_commands(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux open --run-hooks` re-runs post_create hooks."""
    env = isolated_tmux_server
    branch_name = "feature-open-hooks"
    hook_file = "open_hook.txt"

    write_workmux_config(repo_path, post_create=[f"touch {hook_file}"])
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    worktree_path = get_worktree_path(repo_path, branch_name)
    hook_path = worktree_path / hook_file
    hook_path.unlink()

    _kill_window(env, branch_name)

    run_workmux_open(
        env,
        workmux_exe_path,
        repo_path,
        branch_name,
        run_hooks=True,
    )

    assert hook_path.exists()


def test_open_with_force_files_reapplies_file_operations(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux open --force-files` reapplies copy operations."""
    env = isolated_tmux_server
    branch_name = "feature-open-files"
    shared_file = repo_path / "shared.env"
    shared_file.write_text("KEY=value")

    write_workmux_config(repo_path, files={"copy": ["shared.env"]})
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    worktree_path = get_worktree_path(repo_path, branch_name)
    worktree_file = worktree_path / "shared.env"
    worktree_file.unlink()

    _kill_window(env, branch_name)

    run_workmux_open(
        env,
        workmux_exe_path,
        repo_path,
        branch_name,
        force_files=True,
    )

    assert worktree_file.exists()
    assert worktree_file.read_text() == "KEY=value"


# =============================================================================
# Close command tests with duplicate windows
# =============================================================================


def test_close_in_duplicate_window_closes_correct_window(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux close` in a duplicate window closes only that window."""
    env = isolated_tmux_server
    branch_name = "feature-close-dup"
    base_window = get_window_name(branch_name)
    dup_window = f"{base_window}-2"

    write_workmux_config(repo_path)
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    # Create a duplicate window
    run_workmux_open(env, workmux_exe_path, repo_path, branch_name, new_window=True)

    # Verify both windows exist
    list_windows = _get_all_windows(env)
    assert base_window in list_windows
    assert dup_window in list_windows

    # Send close command directly to the duplicate window's pane using send-keys
    # This properly sets TMUX_PANE environment variable unlike run-shell
    # Use session:=window format for exact window name matching
    worktree_path = get_worktree_path(repo_path, branch_name)
    env.tmux(
        [
            "send-keys",
            "-t",
            f"test:={dup_window}",
            f"cd {worktree_path} && {workmux_exe_path} close",
            "Enter",
        ]
    )

    # Wait for the duplicate window to disappear
    def window_gone():
        return dup_window not in _get_all_windows(env)

    assert poll_until(window_gone, timeout=5.0), "Duplicate window should be closed"

    # Verify original window still exists
    list_windows = _get_all_windows(env)
    assert base_window in list_windows, "Original window should still exist"


def test_remove_closes_all_duplicate_windows(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux remove` closes all duplicate windows for a worktree."""
    env = isolated_tmux_server
    branch_name = "feature-remove-dups"
    base_window = get_window_name(branch_name)

    write_workmux_config(repo_path)
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    # Create multiple duplicate windows
    run_workmux_open(env, workmux_exe_path, repo_path, branch_name, new_window=True)
    run_workmux_open(env, workmux_exe_path, repo_path, branch_name, new_window=True)

    # Verify all windows exist
    list_windows = _get_all_windows(env)
    assert base_window in list_windows
    assert f"{base_window}-2" in list_windows
    assert f"{base_window}-3" in list_windows

    # Remove the worktree
    run_workmux_remove(env, workmux_exe_path, repo_path, branch_name, force=True)

    # Verify all windows are closed
    list_windows = _get_all_windows(env)
    matching = [w for w in list_windows if w.startswith(base_window)]
    assert len(matching) == 0, f"All windows should be closed, but found: {matching}"


# =============================================================================
# Prompt support tests
# =============================================================================


def test_open_with_inline_prompt(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux open -p` passes prompt to the worktree."""
    env = isolated_tmux_server
    branch_name = "feature-open-prompt"
    prompt_text = "Fix the login bug"

    # Configure with an agent placeholder that will receive the prompt
    write_workmux_config(repo_path, panes=[{"command": "<agent>"}], agent="claude")
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    # Kill the window
    _kill_window(env, branch_name)

    # Open with a prompt
    result = run_workmux_open(
        env, workmux_exe_path, repo_path, branch_name, prompt=prompt_text
    )

    assert result.exit_code == 0

    # Check that a prompt file was created in the temp directory
    prompt_files = list(env.tmp_path.glob("workmux-prompt-*.md"))
    assert len(prompt_files) >= 1, "Prompt file should have been created"

    # Verify at least one prompt file contains our text
    found_prompt = False
    for pf in prompt_files:
        if prompt_text in pf.read_text():
            found_prompt = True
            break
    assert found_prompt, f"Prompt text not found in any prompt file: {prompt_files}"


def test_open_with_special_characters_in_prompt(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies prompt handling with special characters (quotes, $VAR, backticks)."""
    env = isolated_tmux_server
    branch_name = "feature-special-prompt"
    # Prompt with quotes, dollar signs, backticks, and newlines
    prompt_text = "Refactor: 'Module' needs $FIX.\nVerify `code` behavior."

    write_workmux_config(repo_path, panes=[{"command": "<agent>"}], agent="claude")
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    _kill_window(env, branch_name)

    result = run_workmux_open(
        env, workmux_exe_path, repo_path, branch_name, prompt=prompt_text
    )

    assert result.exit_code == 0

    # Verify the exact prompt text was preserved in the file
    prompt_files = list(env.tmp_path.glob("workmux-prompt-*.md"))
    assert len(prompt_files) >= 1, "Prompt file should have been created"

    found_exact = False
    for pf in prompt_files:
        if prompt_text in pf.read_text():
            found_exact = True
            break
    assert found_exact, (
        "Exact prompt text with special characters not found in any prompt file"
    )


def test_open_from_inside_worktree_switches_to_other(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux open` from inside one worktree can switch to another."""
    env = isolated_tmux_server
    branch_a = "feature-source"
    branch_b = "feature-target"
    window_a = get_window_name(branch_a)
    window_b = get_window_name(branch_b)

    write_workmux_config(repo_path)
    run_workmux_add(env, workmux_exe_path, repo_path, branch_a)
    run_workmux_add(env, workmux_exe_path, repo_path, branch_b)

    # Kill window B to simulate a detached worktree
    env.tmux(["kill-window", "-t", f"test:={window_b}"])

    # Run open from inside window A to open window B
    worktree_a_path = get_worktree_path(repo_path, branch_a)
    env.tmux(
        [
            "send-keys",
            "-t",
            f"test:={window_a}",
            f"cd {worktree_a_path} && {workmux_exe_path} open {branch_b}",
            "Enter",
        ]
    )

    # Wait for window B to appear
    def window_b_exists():
        return window_b in _get_all_windows(env)

    assert poll_until(window_b_exists, timeout=5.0), "Window B should be opened"

    # Both windows should exist
    list_windows = _get_all_windows(env)
    assert window_a in list_windows, "Window A should still exist"
    assert window_b in list_windows, "Window B should be opened"


def test_open_with_prompt_file(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux open -P` reads prompt from file."""
    env = isolated_tmux_server
    branch_name = "feature-open-prompt-file"
    prompt_text = "Implement the new feature\n\nDetails here."

    # Create a prompt file
    prompt_file = repo_path / "my-prompt.md"
    prompt_file.write_text(prompt_text)

    # Configure with an agent placeholder that will receive the prompt
    write_workmux_config(repo_path, panes=[{"command": "<agent>"}], agent="claude")
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    # Kill the window
    _kill_window(env, branch_name)

    # Open with prompt file
    result = run_workmux_open(
        env, workmux_exe_path, repo_path, branch_name, prompt_file=prompt_file
    )

    assert result.exit_code == 0

    # Verify the prompt content was processed into a temp prompt file
    temp_prompt_files = list(env.tmp_path.glob("workmux-prompt-*.md"))
    assert len(temp_prompt_files) >= 1, "Prompt file should have been created"

    # Verify at least one temp file contains our prompt content
    found_content = False
    for pf in temp_prompt_files:
        if "Implement the new feature" in pf.read_text():
            found_content = True
            break
    assert found_content, (
        "Prompt file content was not processed into a temp prompt file"
    )
