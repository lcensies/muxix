"""Tests for `workmux setup` command."""

import json
import shlex
from pathlib import Path


from .conftest import (
    MuxEnvironment,
    get_scripts_dir,
    make_env_script,
    poll_until_file_has_content,
    run_workmux_command,
    wait_for_pane_output,
)


def run_setup_interactive(env: MuxEnvironment, workmux_exe_path: Path) -> Path:
    """Run `workmux setup` interactively in the tmux pane.

    Returns path to exit code file. The setup output is visible in the pane,
    allowing send_keys to respond to prompts.
    """
    scripts_dir = get_scripts_dir(env)
    exit_code_file = scripts_dir / "setup_exit_code.txt"
    if exit_code_file.exists():
        exit_code_file.unlink()

    script = make_env_script(
        env,
        (
            f"{shlex.quote(str(workmux_exe_path))} setup; "
            f"echo $? > {shlex.quote(str(exit_code_file))}"
        ),
        {
            "PATH": env.env["PATH"],
            "HOME": env.env.get("HOME", ""),
            "TMPDIR": env.env.get("TMPDIR", "/tmp"),
            "XDG_CONFIG_HOME": env.env.get("XDG_CONFIG_HOME", ""),
            "XDG_STATE_HOME": env.env.get("XDG_STATE_HOME", ""),
        },
    )
    env.send_keys("test:", script, enter=True)
    return exit_code_file


# ---------------------------------------------------------------------------
# Non-interactive tests (no prompt expected)
# ---------------------------------------------------------------------------


class TestSetupNoPrompt:
    """Tests where setup exits without prompting for input."""

    def test_no_agents_detected(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Prints message when no agent directories exist."""
        result = run_workmux_command(mux_server, workmux_exe_path, repo_path, "setup")
        assert "No agents detected" in result.stdout

    def test_claude_hooks_already_configured(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Claude with manual hooks shows all-configured message."""
        claude_dir = mux_server.home_path / ".claude"
        claude_dir.mkdir()
        settings = {
            "hooks": {
                "Stop": [
                    {
                        "hooks": [
                            {
                                "type": "command",
                                "command": "workmux set-window-status done",
                            }
                        ]
                    }
                ]
            }
        }
        (claude_dir / "settings.json").write_text(json.dumps(settings))

        result = run_workmux_command(mux_server, workmux_exe_path, repo_path, "setup")
        assert "All agents have status tracking configured" in result.stdout

    def test_claude_plugin_enabled(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Claude with enabled plugin shows all-configured message."""
        claude_dir = mux_server.home_path / ".claude"
        claude_dir.mkdir()
        settings = {"enabledPlugins": {"workmux-status@workmux": True}}
        (claude_dir / "settings.json").write_text(json.dumps(settings))

        result = run_workmux_command(mux_server, workmux_exe_path, repo_path, "setup")
        assert "All agents have status tracking configured" in result.stdout

    def test_claude_plugin_disabled_counts_as_configured(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Disabled plugin still counts as configured (user knows about it)."""
        claude_dir = mux_server.home_path / ".claude"
        claude_dir.mkdir()
        settings = {"enabledPlugins": {"workmux-status@workmux": False}}
        (claude_dir / "settings.json").write_text(json.dumps(settings))

        result = run_workmux_command(mux_server, workmux_exe_path, repo_path, "setup")
        assert "All agents have status tracking configured" in result.stdout

    def test_opencode_plugin_configured(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """OpenCode with plugin file shows all-configured message."""
        plugin_dir = mux_server.home_path / ".config" / "opencode" / "plugin"
        plugin_dir.mkdir(parents=True)
        (plugin_dir / "workmux-status.ts").write_text("// plugin")

        result = run_workmux_command(mux_server, workmux_exe_path, repo_path, "setup")
        assert "All agents have status tracking configured" in result.stdout

    def test_both_agents_configured(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Both agents configured shows all-configured message."""
        # Claude
        claude_dir = mux_server.home_path / ".claude"
        claude_dir.mkdir()
        settings = {
            "hooks": {
                "Stop": [
                    {
                        "hooks": [
                            {
                                "type": "command",
                                "command": "workmux set-window-status done",
                            }
                        ]
                    }
                ]
            }
        }
        (claude_dir / "settings.json").write_text(json.dumps(settings))
        # OpenCode
        plugin_dir = mux_server.home_path / ".config" / "opencode" / "plugin"
        plugin_dir.mkdir(parents=True)
        (plugin_dir / "workmux-status.ts").write_text("// plugin")

        result = run_workmux_command(mux_server, workmux_exe_path, repo_path, "setup")
        assert "All agents have status tracking configured" in result.stdout

    def test_requires_interactive_terminal(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Fails when stdin is piped (not a terminal)."""
        result = run_workmux_command(
            mux_server,
            workmux_exe_path,
            repo_path,
            "setup",
            stdin_input="y\n",
            expect_fail=True,
        )
        assert "interactive terminal" in result.stderr


# ---------------------------------------------------------------------------
# Interactive tests (prompt for Y/n)
# ---------------------------------------------------------------------------


class TestSetupInstall:
    """Tests that exercise the interactive install prompt."""

    def test_claude_install_accept(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Answering 'y' installs hooks into settings.json."""
        claude_dir = mux_server.home_path / ".claude"
        claude_dir.mkdir()

        exit_code_file = run_setup_interactive(mux_server, workmux_exe_path)
        wait_for_pane_output(
            mux_server, "test", "Install status tracking hooks?", timeout=5.0
        )
        mux_server.send_keys("test:", "y")

        assert poll_until_file_has_content(exit_code_file, timeout=5.0)
        assert exit_code_file.read_text().strip() == "0"

        settings_path = claude_dir / "settings.json"
        assert settings_path.exists()
        settings = json.loads(settings_path.read_text())
        assert "hooks" in settings
        assert "UserPromptSubmit" in settings["hooks"]
        assert "Notification" in settings["hooks"]
        assert "PostToolUse" in settings["hooks"]
        assert "Stop" in settings["hooks"]

    def test_claude_install_default_enter(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Pressing Enter accepts installation (default is Y)."""
        claude_dir = mux_server.home_path / ".claude"
        claude_dir.mkdir()

        exit_code_file = run_setup_interactive(mux_server, workmux_exe_path)
        wait_for_pane_output(
            mux_server, "test", "Install status tracking hooks?", timeout=5.0
        )
        mux_server.send_keys("test:", "")  # Just Enter

        assert poll_until_file_has_content(exit_code_file, timeout=5.0)
        assert exit_code_file.read_text().strip() == "0"

        settings_path = claude_dir / "settings.json"
        assert settings_path.exists()
        settings = json.loads(settings_path.read_text())
        assert "hooks" in settings

    def test_claude_install_decline(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Answering 'n' skips installation."""
        claude_dir = mux_server.home_path / ".claude"
        claude_dir.mkdir()

        exit_code_file = run_setup_interactive(mux_server, workmux_exe_path)
        wait_for_pane_output(
            mux_server, "test", "Install status tracking hooks?", timeout=5.0
        )
        mux_server.send_keys("test:", "n")

        assert poll_until_file_has_content(exit_code_file, timeout=5.0)
        assert exit_code_file.read_text().strip() == "0"

        settings_path = claude_dir / "settings.json"
        assert not settings_path.exists()

    def test_opencode_install_accept(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Accepting installs OpenCode plugin file."""
        opencode_dir = mux_server.home_path / ".config" / "opencode"
        opencode_dir.mkdir(parents=True)

        exit_code_file = run_setup_interactive(mux_server, workmux_exe_path)
        wait_for_pane_output(
            mux_server, "test", "Install status tracking hooks?", timeout=5.0
        )
        mux_server.send_keys("test:", "y")

        assert poll_until_file_has_content(exit_code_file, timeout=5.0)
        assert exit_code_file.read_text().strip() == "0"

        plugin_path = opencode_dir / "plugin" / "workmux-status.ts"
        assert plugin_path.exists()
        assert len(plugin_path.read_text()) > 0

    def test_both_agents_install_accept(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Accepting installs both agents' hooks."""
        claude_dir = mux_server.home_path / ".claude"
        claude_dir.mkdir()
        opencode_dir = mux_server.home_path / ".config" / "opencode"
        opencode_dir.mkdir(parents=True)

        exit_code_file = run_setup_interactive(mux_server, workmux_exe_path)
        wait_for_pane_output(
            mux_server, "test", "Install status tracking hooks?", timeout=5.0
        )
        mux_server.send_keys("test:", "y")

        assert poll_until_file_has_content(exit_code_file, timeout=5.0)
        assert exit_code_file.read_text().strip() == "0"

        # Claude hooks installed
        settings_path = claude_dir / "settings.json"
        assert settings_path.exists()
        settings = json.loads(settings_path.read_text())
        assert "hooks" in settings
        assert "Stop" in settings["hooks"]

        # OpenCode plugin installed
        plugin_path = opencode_dir / "plugin" / "workmux-status.ts"
        assert plugin_path.exists()

    def test_claude_preserves_existing_settings(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Installing hooks preserves existing settings.json content."""
        claude_dir = mux_server.home_path / ".claude"
        claude_dir.mkdir()
        existing = {
            "permissions": {"allow": ["Bash"]},
            "hooks": {
                "Stop": [
                    {
                        "hooks": [
                            {
                                "type": "command",
                                "command": "afplay /System/Library/Sounds/Glass.aiff",
                            }
                        ]
                    }
                ]
            },
        }
        (claude_dir / "settings.json").write_text(json.dumps(existing, indent=2))

        exit_code_file = run_setup_interactive(mux_server, workmux_exe_path)
        wait_for_pane_output(
            mux_server, "test", "Install status tracking hooks?", timeout=5.0
        )
        mux_server.send_keys("test:", "y")

        assert poll_until_file_has_content(exit_code_file, timeout=5.0)
        assert exit_code_file.read_text().strip() == "0"

        settings = json.loads((claude_dir / "settings.json").read_text())
        # Existing non-hook settings preserved
        assert "permissions" in settings
        assert settings["permissions"]["allow"] == ["Bash"]
        # Existing hooks preserved
        stop_commands = [
            hook.get("command", "")
            for group in settings["hooks"]["Stop"]
            for hook in group.get("hooks", [])
        ]
        assert "afplay /System/Library/Sounds/Glass.aiff" in stop_commands
        # Workmux hooks added alongside existing
        assert "workmux set-window-status done" in stop_commands
        # New hook events added
        assert "UserPromptSubmit" in settings["hooks"]
        assert "Notification" in settings["hooks"]
        assert "PostToolUse" in settings["hooks"]
