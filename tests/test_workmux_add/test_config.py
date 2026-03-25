"""Tests for config file precedence and global/project config merging."""

from pathlib import Path

from ..conftest import (
    FakeAgentInstaller,
    MuxEnvironment,
    RepoBuilder,
    assert_copied_file,
    assert_symlink_to,
    assert_window_exists,
    create_commit,
    file_for_commit,
    get_window_name,
    wait_for_file,
    wait_for_pane_output,
    write_global_workmux_config,
    write_workmux_config,
)
from .conftest import add_branch_and_get_worktree


class TestConfigPrecedence:
    """Tests for project config overriding global config."""

    def test_project_config_overrides_global_config(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
    ):
        """Project-level settings should override conflicting global settings."""
        env = mux_server
        branch_name = "feature-project-overrides"
        global_prefix = "global-"
        project_prefix = "project-"

        write_global_workmux_config(env, window_prefix=global_prefix)
        write_workmux_config(mux_repo_path, window_prefix=project_prefix)

        add_branch_and_get_worktree(env, workmux_exe_path, mux_repo_path, branch_name)

        project_window = f"{project_prefix}{branch_name}"
        assert_window_exists(env, project_window)

        existing_windows = env.list_windows()
        assert f"{global_prefix}{branch_name}" not in existing_windows

    def test_global_config_used_when_project_config_absent(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
    ):
        """Global config should be respected even if the repository lacks .workmux.yaml."""
        env = mux_server
        branch_name = "feature-global-only"
        hook_file = "global_only_hook.txt"

        write_global_workmux_config(env, post_create=[f"touch {hook_file}"])

        worktree_path = add_branch_and_get_worktree(
            env, workmux_exe_path, mux_repo_path, branch_name
        )
        assert (worktree_path / hook_file).exists()


class TestGlobalPlaceholderPostCreate:
    """Tests for <global> placeholder in post_create hooks."""

    def test_global_placeholder_merges_post_create_commands(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
    ):
        """The '<global>' placeholder should expand to global post_create commands."""
        env = mux_server
        branch_name = "feature-global-hooks"
        global_hook = "created_from_global.txt"
        before_hook = "project_before.txt"
        after_hook = "project_after.txt"

        write_global_workmux_config(env, post_create=[f"touch {global_hook}"])
        write_workmux_config(
            mux_repo_path,
            post_create=[f"touch {before_hook}", "<global>", f"touch {after_hook}"],
        )

        worktree_dir = add_branch_and_get_worktree(
            env, workmux_exe_path, mux_repo_path, branch_name
        )
        assert (worktree_dir / before_hook).exists()
        assert (worktree_dir / global_hook).exists()
        assert (worktree_dir / after_hook).exists()


class TestGlobalPlaceholderFiles:
    """Tests for <global> placeholder in file operations."""

    def test_global_placeholder_merges_file_operations(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
        repo_builder: RepoBuilder,
    ):
        """The '<global>' placeholder should merge copy and symlink file operations."""
        env = mux_server
        branch_name = "feature-global-files"

        # Create files/directories that will be copied or symlinked.
        repo_builder.with_files(
            {
                "global.env": "GLOBAL",
                "project.env": "PROJECT",
                "global_cache/shared.txt": "global data",
                "project_cache/local.txt": "project data",
            }
        ).commit("Add files for global placeholder tests")

        write_global_workmux_config(
            env,
            files={"copy": ["global.env"], "symlink": ["global_cache"]},
        )
        write_workmux_config(
            mux_repo_path,
            files={
                "copy": ["<global>", "project.env"],
                "symlink": ["<global>", "project_cache"],
            },
        )

        worktree_dir = add_branch_and_get_worktree(
            env, workmux_exe_path, mux_repo_path, branch_name
        )
        symlinked_global = assert_symlink_to(worktree_dir, "global_cache")
        symlinked_project = assert_symlink_to(worktree_dir, "project_cache")
        assert (symlinked_global / "shared.txt").read_text() == "global data"
        assert (symlinked_project / "local.txt").read_text() == "project data"

        assert_copied_file(worktree_dir, "global.env", "GLOBAL")
        assert_copied_file(worktree_dir, "project.env", "PROJECT")

    def test_global_placeholder_only_merges_specific_file_lists(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
        repo_builder: RepoBuilder,
    ):
        """`<global>` can merge copy patterns while symlink patterns fully override."""
        env = mux_server
        branch_name = "feature-partial-file-merge"

        repo_builder.add_to_gitignore(
            [
                "global_copy.txt",
                "project_copy.txt",
                "global_symlink_dir/",
                "project_symlink_dir/",
            ]
        )

        (mux_repo_path / "global_copy.txt").write_text("global copy")
        (mux_repo_path / "project_copy.txt").write_text("project copy")
        global_symlink_dir = mux_repo_path / "global_symlink_dir"
        global_symlink_dir.mkdir()
        (global_symlink_dir / "global.txt").write_text("global data")
        project_symlink_dir = mux_repo_path / "project_symlink_dir"
        project_symlink_dir.mkdir()
        (project_symlink_dir / "project.txt").write_text("project data")

        write_global_workmux_config(
            env,
            files={"copy": ["global_copy.txt"], "symlink": ["global_symlink_dir"]},
        )
        write_workmux_config(
            mux_repo_path,
            files={
                "copy": ["<global>", "project_copy.txt"],
                "symlink": ["project_symlink_dir"],
            },
        )

        worktree_dir = add_branch_and_get_worktree(
            env, workmux_exe_path, mux_repo_path, branch_name
        )
        assert_copied_file(worktree_dir, "global_copy.txt")
        assert_copied_file(worktree_dir, "project_copy.txt")

        assert_symlink_to(worktree_dir, "project_symlink_dir")
        assert not (worktree_dir / "global_symlink_dir").exists()


class TestEmptyOverrides:
    """Tests for empty lists overriding global config."""

    def test_project_empty_file_lists_override_global_lists(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
        repo_builder: RepoBuilder,
    ):
        """Explicit empty lists suppress the corresponding global file operations."""
        env = mux_server
        branch_name = "feature-empty-file-override"

        repo_builder.add_to_gitignore(
            [
                "global_only.env",
                "global_shared_dir/",
            ]
        )

        (mux_repo_path / "global_only.env").write_text("SECRET=1")
        global_shared_dir = mux_repo_path / "global_shared_dir"
        global_shared_dir.mkdir()
        (global_shared_dir / "package.json").write_text('{"name":"demo"}')

        write_global_workmux_config(
            env,
            files={"copy": ["global_only.env"], "symlink": ["global_shared_dir"]},
        )
        write_workmux_config(
            mux_repo_path,
            files={"copy": [], "symlink": []},
        )

        worktree_dir = add_branch_and_get_worktree(
            env, workmux_exe_path, mux_repo_path, branch_name
        )
        assert not (worktree_dir / "global_only.env").exists()
        assert not (worktree_dir / "global_shared_dir").exists()


class TestPaneOverrides:
    """Tests for pane config overrides."""

    def test_project_panes_replace_global_panes(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
    ):
        """Project panes should completely replace global panes (no merging)."""
        env = mux_server
        branch_name = "feature-pane-override"
        window_name = get_window_name(branch_name)
        global_output = "GLOBAL_PANE_OUTPUT"
        project_output = "PROJECT_PANE_OUTPUT"

        write_global_workmux_config(
            env, panes=[{"command": f"echo '{global_output}'; sleep 0.5"}]
        )
        write_workmux_config(
            mux_repo_path, panes=[{"command": f"echo '{project_output}'; sleep 0.5"}]
        )

        add_branch_and_get_worktree(env, workmux_exe_path, mux_repo_path, branch_name)

        wait_for_pane_output(env, window_name, project_output)

        pane_content = env.capture_pane(window_name)
        assert pane_content is not None
        assert global_output not in pane_content


class TestGlobalAgentDefault:
    """Tests for global agent config triggering agent-aware default panes."""

    def test_global_agent_starts_in_default_pane(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
        fake_agent_installer: FakeAgentInstaller,
    ):
        """When agent is set in global config and no panes are defined, the agent should run."""
        env = mux_server
        branch_name = "feature-global-agent-default"
        window_name = get_window_name(branch_name)

        # Ensure CLAUDE.md does not exist so we isolate the global agent config behavior
        assert not (mux_repo_path / "CLAUDE.md").exists()

        # Use absolute path for output to avoid cwd/shell-init races
        agent_output = env.tmp_path / "global_agent_ran.txt"

        # Install fake agent; use absolute path for both agent command and output
        # to avoid PATH resolution issues when the login shell re-initializes PATH
        agent_path = fake_agent_installer.install(
            "global_agent",
            f"#!/bin/sh\necho ran > {agent_output}\n",
        )

        # Write global config with absolute agent path but NO explicit panes
        write_global_workmux_config(env, agent=str(agent_path))

        # Do NOT write project-level .workmux.yaml

        worktree_path = add_branch_and_get_worktree(
            env, workmux_exe_path, mux_repo_path, branch_name
        )

        wait_for_file(
            env,
            agent_output,
            timeout=10.0,
            window_name=window_name,
            worktree_path=worktree_path,
        )
        assert agent_output.read_text().strip() == "ran"


class TestBaseBranchConfig:
    """Tests for base_branch config option."""

    def test_config_base_branch_used_when_base_flag_omitted(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
    ):
        """Config base_branch should be used as default when --base is not passed."""
        env = mux_server
        base_branch = "config-base-source"
        new_branch = "feature-from-config-base"
        commit_msg = "Commit on config base"

        # Create a branch with a unique commit
        env.run_command(["git", "checkout", "-b", base_branch], cwd=mux_repo_path)
        create_commit(env, mux_repo_path, commit_msg)

        # Go back to main so the current branch is NOT the base
        env.run_command(["git", "checkout", "main"], cwd=mux_repo_path)

        # Write config with base_branch pointing to our branch
        write_workmux_config(mux_repo_path, base_branch=base_branch)

        # Add without --base; config should kick in
        worktree_path = add_branch_and_get_worktree(
            env, workmux_exe_path, mux_repo_path, new_branch
        )

        # The new worktree should have the commit from the configured base branch
        expected_file = file_for_commit(worktree_path, commit_msg)
        assert expected_file.exists()

    def test_cli_base_overrides_config_base_branch(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
    ):
        """CLI --base flag should override config base_branch."""
        env = mux_server
        config_base = "config-base-override"
        cli_base = "cli-base-override"
        new_branch = "feature-cli-overrides-config"
        config_commit = "Commit on config base branch"
        cli_commit = "Commit on cli base branch"

        # Create the config base branch with a commit
        env.run_command(["git", "checkout", "-b", config_base], cwd=mux_repo_path)
        create_commit(env, mux_repo_path, config_commit)

        # Create the CLI base branch with a different commit
        env.run_command(["git", "checkout", "main"], cwd=mux_repo_path)
        env.run_command(["git", "checkout", "-b", cli_base], cwd=mux_repo_path)
        create_commit(env, mux_repo_path, cli_commit)

        # Go back to main
        env.run_command(["git", "checkout", "main"], cwd=mux_repo_path)

        # Config points to config_base, but we pass --base cli_base
        write_workmux_config(mux_repo_path, base_branch=config_base)

        worktree_path = add_branch_and_get_worktree(
            env,
            workmux_exe_path,
            mux_repo_path,
            new_branch,
            extra_args=f"--base {cli_base}",
        )

        # Should have the CLI base commit, not the config base commit
        assert file_for_commit(worktree_path, cli_commit).exists()
        assert not file_for_commit(worktree_path, config_commit).exists()
