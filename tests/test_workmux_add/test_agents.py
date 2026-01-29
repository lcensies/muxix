"""Tests for agent configuration, prompts, and multi-agent scenarios."""

import shlex
from pathlib import Path


from ..conftest import (
    MuxEnvironment,
    FakeAgentInstaller,
    ShellCommands,
    assert_prompt_file_contents,
    assert_window_exists,
    get_window_name,
    get_worktree_path,
    poll_until,
    run_workmux_command,
    wait_for_file,
    write_workmux_config,
)
from .conftest import add_branch_and_get_worktree


class TestInlinePrompts:
    """Tests for inline prompt injection into agents."""

    def test_add_inline_prompt_injects_into_claude(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
        fake_agent_installer: FakeAgentInstaller,
        shell_cmd: ShellCommands,
    ):
        """Inline prompts should be written to PROMPT.md and passed to claude via command substitution."""
        env = mux_server
        branch_name = "feature-inline-prompt"
        prompt_text = "Implement inline prompt"
        output_filename = "claude_prompt.txt"
        window_name = get_window_name(branch_name)

        # Configure the shell
        env.configure_default_shell(shell_cmd.path)

        fake_agent_installer.install(
            "claude",
            f"""#!/bin/sh
# Debug: log all arguments
echo "ARGS: $@" > debug_args.txt
echo "ARG1: $1" >> debug_args.txt
echo "ARG2: $2" >> debug_args.txt

set -e
# The implementation calls: claude -- "$(cat PROMPT.md)"
# So we expect -- as $1 and the prompt content as the second argument
printf '%s' "$2" > "{output_filename}"
""",
        )

        # Write RC file with PATH to fake agent
        rc_path = env.home_path / shell_cmd.rc_filename
        rc_path.parent.mkdir(parents=True, exist_ok=True)
        rc_path.write_text(
            shell_cmd.prepend_path(str(fake_agent_installer.bin_dir)) + "\n"
        )

        # Use agent name - shell will find it via PATH from RC file
        write_workmux_config(
            mux_repo_path, agent="claude", panes=[{"command": "<agent>"}]
        )

        worktree_path = add_branch_and_get_worktree(
            env,
            workmux_exe_path,
            mux_repo_path,
            branch_name,
            extra_args=f"--prompt {shlex.quote(prompt_text)}",
        )

        # Prompt file is now written to the test's temp directory
        assert_prompt_file_contents(env, branch_name, prompt_text)

        agent_output = worktree_path / output_filename
        debug_output = worktree_path / "debug_args.txt"

        wait_for_file(
            env,
            agent_output,
            timeout=5.0,  # Increased for slower shells
            window_name=window_name,
            worktree_path=worktree_path,
            debug_log_path=debug_output,
        )

        assert agent_output.read_text() == prompt_text


class TestPromptFile:
    """Tests for file-based prompt injection."""

    def test_add_prompt_file_injects_into_gemini(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
        fake_agent_installer: FakeAgentInstaller,
        shell_cmd: ShellCommands,
    ):
        """Prompt file flag should populate PROMPT.md and pass it to gemini via command substitution."""
        env = mux_server
        branch_name = "feature-file-prompt"
        window_name = get_window_name(branch_name)
        prompt_source = mux_repo_path / "prompt_source.txt"
        prompt_source.write_text("File-based instructions")
        output_filename = "gemini_prompt.txt"

        # Configure the shell
        env.configure_default_shell(shell_cmd.path)

        fake_agent_installer.install(
            "gemini",
            f"""#!/bin/sh
set -e
# The implementation calls: gemini -i "$(cat PROMPT.md)"
# So we expect -i flag first, then the prompt content as the second argument
if [ "$1" != "-i" ]; then
    echo "Expected -i flag first" >&2
    exit 1
fi
printf '%s' "$2" > "{output_filename}"
""",
        )

        # Write RC file with PATH to fake agent
        rc_path = env.home_path / shell_cmd.rc_filename
        rc_path.parent.mkdir(parents=True, exist_ok=True)
        rc_path.write_text(
            shell_cmd.prepend_path(str(fake_agent_installer.bin_dir)) + "\n"
        )

        # Use agent name - shell will find it via PATH from RC file
        write_workmux_config(
            mux_repo_path, agent="gemini", panes=[{"command": "<agent>"}]
        )

        worktree_path = add_branch_and_get_worktree(
            env,
            workmux_exe_path,
            mux_repo_path,
            branch_name,
            extra_args=f"--prompt-file {shlex.quote(str(prompt_source))}",
        )

        # Prompt file is now written to the test's temp directory
        assert_prompt_file_contents(env, branch_name, prompt_source.read_text())

        agent_output = worktree_path / output_filename

        wait_for_file(
            env,
            agent_output,
            timeout=5.0,  # Increased for slower shells
            window_name=window_name,
            worktree_path=worktree_path,
        )
        assert agent_output.read_text() == prompt_source.read_text()


class TestAgentConfig:
    """Tests for agent configuration from config file."""

    def test_add_uses_agent_from_config(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
        fake_agent_installer: FakeAgentInstaller,
        shell_cmd: ShellCommands,
    ):
        """The <agent> placeholder should use the agent configured in .workmux.yaml when --agent is not passed."""
        env = mux_server
        branch_name = "feature-config-agent"
        window_name = get_window_name(branch_name)
        prompt_text = "Using configured agent"
        output_filename = "agent_output.txt"

        # Configure the shell
        env.configure_default_shell(shell_cmd.path)

        # Install fake gemini agent
        fake_agent_installer.install(
            "gemini",
            f"""#!/bin/sh
set -e
# Gemini gets a -i flag, then the prompt as $2
printf '%s' "$2" > "{output_filename}"
""",
        )

        # Write RC file with PATH to fake agent
        rc_path = env.home_path / shell_cmd.rc_filename
        rc_path.parent.mkdir(parents=True, exist_ok=True)
        rc_path.write_text(
            shell_cmd.prepend_path(str(fake_agent_installer.bin_dir)) + "\n"
        )

        # Configure .workmux.yaml to use the agent name (found via PATH)
        write_workmux_config(
            mux_repo_path, agent="gemini", panes=[{"command": "<agent>"}]
        )

        # Run 'add' WITHOUT --agent flag, should use gemini from config
        worktree_path = add_branch_and_get_worktree(
            env,
            workmux_exe_path,
            mux_repo_path,
            branch_name,
            extra_args=f"--prompt {shlex.quote(prompt_text)}",
        )

        agent_output = worktree_path / output_filename

        wait_for_file(
            env,
            agent_output,
            timeout=5.0,  # Increased for slower shells
            window_name=window_name,
            worktree_path=worktree_path,
        )
        assert agent_output.read_text() == prompt_text

    def test_add_with_agent_flag_overrides_default(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
        fake_agent_installer: FakeAgentInstaller,
    ):
        """The --agent flag should override the default agent and inject prompts correctly."""
        env = mux_server
        branch_name = "feature-agent-override"
        window_name = get_window_name(branch_name)
        prompt_text = "This is for the override agent"
        output_filename = "agent_output.txt"

        # Create two fake agents: a default one and the one we'll specify via the flag.
        # Default agent (claude)
        fake_agent_installer.install(
            "claude",
            "#!/bin/sh\necho 'default agent ran' > default_agent.txt",
        )

        # Override agent (gemini)
        fake_gemini_path = fake_agent_installer.install(
            "gemini",
            f"""#!/bin/sh
# Gemini gets a -i flag, then the prompt as $2
printf '%s' "$2" > "{output_filename}"
""",
        )

        # Configure workmux to use <agent> placeholder. The default should be 'claude'.
        write_workmux_config(mux_repo_path, panes=[{"command": "<agent>"}])

        # Run 'add' with the --agent flag to override the default, using absolute path
        worktree_path = add_branch_and_get_worktree(
            env,
            workmux_exe_path,
            mux_repo_path,
            branch_name,
            extra_args=f"--agent {shlex.quote(str(fake_gemini_path))} --prompt {shlex.quote(prompt_text)}",
        )

        agent_output = worktree_path / output_filename
        default_agent_output = worktree_path / "default_agent.txt"

        wait_for_file(
            env,
            agent_output,
            timeout=2.0,
            window_name=window_name,
            worktree_path=worktree_path,
        )
        assert not default_agent_output.exists(), "Default agent should not have run"
        assert agent_output.read_text() == prompt_text


class TestAgentWithArguments:
    """Tests for agent commands that include arguments."""

    def test_agent_with_dangerously_skip_permissions_flag(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
        fake_agent_installer: FakeAgentInstaller,
    ):
        """Agent config with --dangerously-skip-permissions should pass flag and prompt correctly."""
        env = mux_server
        branch_name = "feature-yolo-mode"
        window_name = get_window_name(branch_name)
        prompt_text = "Implement yolo feature"
        output_filename = "agent_output.txt"
        flag_marker = "flag_found.txt"

        # Fake claude that verifies both the flag and prompt are received
        fake_claude_path = fake_agent_installer.install(
            "claude",
            f"""#!/bin/sh
set -e
# Log all args for debugging
echo "ARGS: $@" > debug_args.txt

# Check for --dangerously-skip-permissions flag
flag_found=0
for arg in "$@"; do
    if [ "$arg" = "--dangerously-skip-permissions" ]; then
        flag_found=1
        echo "yes" > "{flag_marker}"
        break
    fi
done

if [ "$flag_found" = "0" ]; then
    echo "no" > "{flag_marker}"
fi

# The command is: claude --dangerously-skip-permissions -- "prompt"
# So after the flag, we expect -- and then the prompt
# Find the prompt (argument after --)
prompt=""
found_separator=0
for arg in "$@"; do
    if [ "$found_separator" = "1" ]; then
        prompt="$arg"
        break
    fi
    if [ "$arg" = "--" ]; then
        found_separator=1
    fi
done

printf '%s' "$prompt" > "{output_filename}"
""",
        )

        # Configure agent with the flag included
        write_workmux_config(
            mux_repo_path,
            agent=f"{fake_claude_path} --dangerously-skip-permissions",
            panes=[{"command": "<agent>"}],
        )

        worktree_path = add_branch_and_get_worktree(
            env,
            workmux_exe_path,
            mux_repo_path,
            branch_name,
            extra_args=f"--prompt {shlex.quote(prompt_text)}",
        )

        agent_output = worktree_path / output_filename
        flag_file = worktree_path / flag_marker
        debug_file = worktree_path / "debug_args.txt"

        wait_for_file(
            env,
            agent_output,
            timeout=2.0,
            window_name=window_name,
            worktree_path=worktree_path,
            debug_log_path=debug_file,
        )

        # Verify the flag was received
        assert flag_file.exists(), "Flag marker file not created"
        assert flag_file.read_text().strip() == "yes", (
            f"--dangerously-skip-permissions flag not found. Debug: {debug_file.read_text() if debug_file.exists() else 'no debug'}"
        )

        # Verify the prompt was received
        assert agent_output.read_text() == prompt_text

    def test_agent_with_multiple_arguments(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
        fake_agent_installer: FakeAgentInstaller,
    ):
        """Agent config with multiple arguments should pass all args and prompt correctly."""
        env = mux_server
        branch_name = "feature-multi-args"
        window_name = get_window_name(branch_name)
        prompt_text = "Multi-arg task"
        output_filename = "agent_output.txt"

        # Fake claude that captures all args before --
        fake_claude_path = fake_agent_installer.install(
            "claude",
            f"""#!/bin/sh
set -e
echo "ARGS: $@" > debug_args.txt

# Collect args before -- separator
args_before=""
prompt=""
found_separator=0
for arg in "$@"; do
    if [ "$found_separator" = "1" ]; then
        prompt="$arg"
        break
    fi
    if [ "$arg" = "--" ]; then
        found_separator=1
    else
        if [ -n "$args_before" ]; then
            args_before="$args_before $arg"
        else
            args_before="$arg"
        fi
    fi
done

echo "$args_before" > args_received.txt
printf '%s' "$prompt" > "{output_filename}"
""",
        )

        # Configure agent with multiple flags
        write_workmux_config(
            mux_repo_path,
            agent=f"{fake_claude_path} --verbose --model opus",
            panes=[{"command": "<agent>"}],
        )

        worktree_path = add_branch_and_get_worktree(
            env,
            workmux_exe_path,
            mux_repo_path,
            branch_name,
            extra_args=f"--prompt {shlex.quote(prompt_text)}",
        )

        agent_output = worktree_path / output_filename
        args_file = worktree_path / "args_received.txt"

        wait_for_file(
            env,
            agent_output,
            timeout=2.0,
            window_name=window_name,
            worktree_path=worktree_path,
        )

        # Verify all args were received
        args_received = args_file.read_text().strip()
        assert "--verbose" in args_received, f"--verbose not found in: {args_received}"
        assert "--model" in args_received, f"--model not found in: {args_received}"
        assert "opus" in args_received, f"opus not found in: {args_received}"

        # Verify the prompt was received
        assert agent_output.read_text() == prompt_text


class TestMultiAgent:
    """Tests for multi-agent scenarios."""

    def test_add_multi_agent_creates_separate_worktrees_and_runs_correct_agents(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
        fake_agent_installer: FakeAgentInstaller,
    ):
        """Verifies `-a` with multiple agents creates distinct worktrees for each agent."""
        env = mux_server
        base_name = "feature-multi-agent"
        prompt_text = "Implement for {{ agent }}"

        claude_path = fake_agent_installer.install(
            "claude",
            "#!/bin/sh\nprintf '%s' \"$2\" > claude_out.txt",
        )
        gemini_path = fake_agent_installer.install(
            "gemini",
            "#!/bin/sh\nprintf '%s' \"$2\" > gemini_out.txt",
        )

        write_workmux_config(mux_repo_path, panes=[{"command": "<agent>"}])

        run_workmux_command(
            env,
            workmux_exe_path,
            mux_repo_path,
            f"add {base_name} -a {shlex.quote(str(claude_path))} -a {shlex.quote(str(gemini_path))} --prompt '{prompt_text}'",
        )

        claude_branch = f"{base_name}-claude"
        claude_worktree = get_worktree_path(mux_repo_path, claude_branch)
        assert claude_worktree.is_dir()
        claude_window = get_window_name(claude_branch)
        assert_window_exists(env, claude_window)
        wait_for_file(
            env,
            claude_worktree / "claude_out.txt",
            window_name=claude_window,
            worktree_path=claude_worktree,
        )
        assert (
            claude_worktree / "claude_out.txt"
        ).read_text() == "Implement for claude"

        gemini_branch = f"{base_name}-gemini"
        gemini_worktree = get_worktree_path(mux_repo_path, gemini_branch)
        assert gemini_worktree.is_dir()
        gemini_window = get_window_name(gemini_branch)
        assert_window_exists(env, gemini_window)
        wait_for_file(
            env,
            gemini_worktree / "gemini_out.txt",
            window_name=gemini_window,
            worktree_path=gemini_worktree,
        )
        assert (
            gemini_worktree / "gemini_out.txt"
        ).read_text() == "Implement for gemini"

    def test_add_with_count_and_agent_uses_agent_in_all_instances(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
        fake_agent_installer: FakeAgentInstaller,
    ):
        """Verifies count with a single agent uses that agent in all generated worktrees."""
        env = mux_server
        base_name = "feature-counted-agent"
        prompt_text = "Task {{ num }}"

        fake_gemini_path = fake_agent_installer.install(
            "gemini",
            '#!/bin/sh\nprintf \'%s\' "$2" > "gemini_task_${HOSTNAME}.txt"',
        )
        write_workmux_config(mux_repo_path, panes=[{"command": "<agent>"}])

        run_workmux_command(
            env,
            workmux_exe_path,
            mux_repo_path,
            f"add {base_name} -a {shlex.quote(str(fake_gemini_path))} -n 2 --prompt '{prompt_text}'",
        )

        for idx in (1, 2):
            branch = f"{base_name}-gemini-{idx}"
            worktree = get_worktree_path(mux_repo_path, branch)
            assert worktree.is_dir()
            files: list[Path] = []

            def _has_output() -> bool:
                files.clear()
                files.extend(worktree.glob("gemini_task_*.txt"))
                return len(files) == 1

            assert poll_until(_has_output, timeout=2.0), (
                f"gemini output file not found in worktree {worktree}"
            )
            assert files[0].read_text() == f"Task {idx}"


class TestForeach:
    """Tests for --foreach matrix expansion."""

    def test_add_foreach_creates_worktrees_from_matrix(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
        fake_agent_installer: FakeAgentInstaller,
    ):
        """Verifies foreach matrix expands into multiple worktrees with templated prompts."""
        env = mux_server
        base_name = "feature-matrix"
        prompt_text = "Build for {{ platform }} using {{ lang }}"

        claude_path = fake_agent_installer.install(
            "claude",
            "#!/bin/sh\nprintf '%s' \"$2\" > out.txt",
        )
        write_workmux_config(
            mux_repo_path, agent=str(claude_path), panes=[{"command": "<agent>"}]
        )

        run_workmux_command(
            env,
            workmux_exe_path,
            mux_repo_path,
            (
                f"add {base_name} --foreach "
                "'platform:ios,android;lang:swift,kotlin' "
                f"--prompt '{prompt_text}'"
            ),
        )

        combos = [
            ("ios", "swift"),
            ("android", "kotlin"),
        ]
        for platform, lang in combos:
            branch = f"{base_name}-{lang}-{platform}"
            worktree = get_worktree_path(mux_repo_path, branch)
            assert worktree.is_dir()
            window = get_window_name(branch)
            assert_window_exists(env, window)
            wait_for_file(
                env,
                worktree / "out.txt",
                window_name=window,
                worktree_path=worktree,
            )
            assert (
                worktree / "out.txt"
            ).read_text() == f"Build for {platform} using {lang}"


class TestBranchTemplate:
    """Tests for --branch-template flag."""

    def test_add_with_custom_branch_template(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
    ):
        """Verifies `--branch-template` controls the branch naming scheme."""
        env = mux_server
        base_name = "TICKET-123"
        template = r"{{ agent }}/{{ base_name | lower }}-{{ num }}"

        write_workmux_config(mux_repo_path)
        run_workmux_command(
            env,
            workmux_exe_path,
            mux_repo_path,
            f"add {base_name} -a Gemini -n 2 --branch-template '{template}'",
        )

        for idx in (1, 2):
            branch = f"gemini/ticket-123-{idx}"
            worktree = get_worktree_path(mux_repo_path, branch)
            assert worktree.is_dir(), f"Worktree {branch} not found"


class TestNoPrompt:
    """Tests for behavior without prompts."""

    def test_add_without_prompt_skips_prompt_file(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
    ):
        """Worktrees created without prompt flags should not create PROMPT.md."""
        env = mux_server
        branch_name = "feature-no-prompt"

        from ..conftest import prompt_file_for_branch

        write_workmux_config(mux_repo_path, panes=[])

        worktree_path = add_branch_and_get_worktree(
            env, workmux_exe_path, mux_repo_path, branch_name
        )
        # Verify no PROMPT.md in worktree
        assert not (worktree_path / "PROMPT.md").exists()
        # Verify no prompt file in temp dir either
        assert not prompt_file_for_branch(env.tmp_path, branch_name).exists()


class TestShellAliases:
    """Tests for shell alias support with agents."""

    def test_agent_placeholder_respects_shell_aliases(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
        fake_agent_installer: FakeAgentInstaller,
        shell_cmd: ShellCommands,
    ):
        """Verifies that the <agent> placeholder triggers aliases defined in shell rc files."""
        env = mux_server
        branch_name = "feature-agent-alias"
        window_name = get_window_name(branch_name)
        marker_content = "alias_was_expanded"

        # Configure the default shell
        env.configure_default_shell(shell_cmd.path)

        # Get the path where the fake agent will be installed
        fake_bin_dir = fake_agent_installer.bin_dir

        # Write shell-appropriate RC file that prepends our fake bin to PATH and defines alias
        rc_path = env.home_path / shell_cmd.rc_filename
        rc_path.parent.mkdir(parents=True, exist_ok=True)
        rc_content = f"""
{shell_cmd.prepend_path(str(fake_bin_dir))}
{shell_cmd.alias("claude", "claude --aliased")}
"""
        rc_path.write_text(rc_content.strip() + "\n")

        fake_agent_installer.install(
            "claude",
            f"""#!/bin/sh
set -e
for arg in "$@"; do
  if [ "$arg" = "--aliased" ]; then
    echo "{marker_content}" > alias_marker.txt
    exit 0
  fi
done
echo "Alias flag not found" > alias_marker.txt
exit 1
""",
        )

        write_workmux_config(
            mux_repo_path, agent="claude", panes=[{"command": "<agent>"}]
        )

        worktree_path = add_branch_and_get_worktree(
            env, workmux_exe_path, mux_repo_path, branch_name
        )
        marker_file = worktree_path / "alias_marker.txt"

        wait_for_file(
            env,
            marker_file,
            timeout=5.0,  # Increased for slower shells like nushell
            window_name=window_name,
            worktree_path=worktree_path,
        )
        assert marker_file.read_text().strip() == marker_content, (
            "Alias marker content incorrect; alias flag not detected."
        )


class TestAgentErrors:
    """Tests for error handling with agent flags."""

    def test_add_fails_with_count_and_multiple_agents(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
    ):
        """Verifies --count cannot be combined with multiple --agent flags."""
        env = mux_server
        result = run_workmux_command(
            env,
            workmux_exe_path,
            mux_repo_path,
            "add my-feature -n 2 -a claude -a gemini",
            expect_fail=True,
        )
        assert "--count can only be used with zero or one --agent" in result.stderr

    def test_add_fails_with_foreach_and_agent(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
    ):
        """Verifies clap rejects --foreach in combination with --agent."""
        env = mux_server
        result = run_workmux_command(
            env,
            workmux_exe_path,
            mux_repo_path,
            "add my-feature --foreach 'p:a' -a claude",
            expect_fail=True,
        )
        assert (
            "'--foreach <FOREACH>' cannot be used with '--agent <AGENT>'"
            in result.stderr
        )

    def test_add_fails_with_foreach_mismatched_lengths(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
    ):
        """Verifies foreach parser enforces equal list lengths."""
        env = mux_server
        result = run_workmux_command(
            env,
            workmux_exe_path,
            mux_repo_path,
            "add my-feature --foreach 'platform:ios,android;lang:swift'",
            expect_fail=True,
        )
        assert (
            "All --foreach variables must have the same number of values"
            in result.stderr
        )

    def test_add_fails_with_prompt_but_no_pane_has_agent_placeholder(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
    ):
        """Verifies -p fails when panes don't include <agent> placeholder and don't run the default agent."""
        env = mux_server
        # Config with no <agent> placeholder - agent defaults to "claude" but no pane runs it
        write_workmux_config(mux_repo_path, panes=[{"command": "clear"}])
        result = run_workmux_command(
            env,
            workmux_exe_path,
            mux_repo_path,
            "add my-feature --prompt 'do something'",
            expect_fail=True,
        )
        # Agent defaults to "claude", so error says no pane runs claude
        assert "no pane is configured to run the agent" in result.stderr
        assert "claude" in result.stderr

    def test_add_fails_with_prompt_but_no_pane_runs_agent(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
    ):
        """Verifies -p fails when panes don't run the configured agent."""
        env = mux_server
        # Config with agent but panes don't use it
        write_workmux_config(
            mux_repo_path,
            agent="claude",
            panes=[{"command": "vim"}, {"command": "clear", "split": "horizontal"}],
        )
        result = run_workmux_command(
            env,
            workmux_exe_path,
            mux_repo_path,
            "add my-feature --prompt 'do something'",
            expect_fail=True,
        )
        assert "no pane is configured to run the agent" in result.stderr
        assert "claude" in result.stderr

    def test_add_fails_with_prompt_and_no_pane_cmds(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
    ):
        """Verifies -p fails when combined with --no-pane-cmds."""
        env = mux_server
        write_workmux_config(mux_repo_path, panes=[{"command": "<agent>"}])
        result = run_workmux_command(
            env,
            workmux_exe_path,
            mux_repo_path,
            "add my-feature --prompt 'do something' --no-pane-cmds",
            expect_fail=True,
        )
        assert "pane commands are disabled" in result.stderr


class TestTemplateVariableValidation:
    """Tests for template variable validation."""

    def test_add_fails_with_undefined_prompt_variable(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
    ):
        """Verifies prompt with undefined template variable fails with helpful error."""
        env = mux_server
        write_workmux_config(mux_repo_path, panes=[{"command": "<agent>"}])
        result = run_workmux_command(
            env,
            workmux_exe_path,
            mux_repo_path,
            "add my-feature --prompt 'Build for {{ undefined_var }}'",
            expect_fail=True,
        )
        assert "undefined variables" in result.stderr
        assert "undefined_var" in result.stderr
        assert "Available variables" in result.stderr

    def test_add_fails_with_undefined_branch_template_variable(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
    ):
        """Verifies branch template with undefined variable fails with helpful error."""
        env = mux_server
        write_workmux_config(mux_repo_path, panes=[])
        result = run_workmux_command(
            env,
            workmux_exe_path,
            mux_repo_path,
            "add my-feature -n 2 --branch-template '{{ base_name }}-{{ typo }}'",
            expect_fail=True,
        )
        assert "Invalid branch name template" in result.stderr
        assert "typo" in result.stderr

    def test_add_succeeds_with_valid_foreach_variables(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
        fake_agent_installer: FakeAgentInstaller,
    ):
        """Verifies foreach variables are available in prompts."""
        env = mux_server
        base_name = "feature-valid-vars"

        claude_path = fake_agent_installer.install(
            "claude",
            "#!/bin/sh\nprintf '%s' \"$2\" > out.txt",
        )
        write_workmux_config(
            mux_repo_path, agent=str(claude_path), panes=[{"command": "<agent>"}]
        )

        # This should succeed because platform and lang are defined by foreach
        run_workmux_command(
            env,
            workmux_exe_path,
            mux_repo_path,
            (
                f"add {base_name} --foreach "
                "'platform:ios;lang:swift' "
                "--prompt 'Build {{ platform }} with {{ lang }}'"
            ),
        )

        # Verify worktree was created
        worktree = get_worktree_path(mux_repo_path, f"{base_name}-swift-ios")
        assert worktree.is_dir()

    def test_add_fails_with_typo_in_foreach_variable_name(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
    ):
        """Verifies typo in foreach variable name fails with helpful error."""
        env = mux_server
        write_workmux_config(mux_repo_path, panes=[{"command": "<agent>"}])
        result = run_workmux_command(
            env,
            workmux_exe_path,
            mux_repo_path,
            (
                "add my-feature --foreach "
                "'platform:ios,android' "
                "--prompt 'Build {{ plattform }}'"  # typo: plattform instead of platform
            ),
            expect_fail=True,
        )
        assert "undefined variables" in result.stderr
        assert "plattform" in result.stderr
        # Should suggest the correct variable
        assert "platform" in result.stderr
