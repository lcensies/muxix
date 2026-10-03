"""
Integration tests for sandbox checkpoint / resume / focus features.

These tests run muxix CLI as a direct subprocess with:
- XDG_STATE_HOME pointing at a temp dir (so we control the agent state store)
- A fake `msb` CLI script that records what commands were called

No tmux session or real microsandbox VM is required.
"""

import json
import os
import shlex
import shutil
import subprocess
import time
from pathlib import Path

import pytest
import yaml

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def muxix_exe() -> Path:
    """Locate the muxix binary built for this branch."""
    candidate = Path(__file__).parent.parent / "target" / "debug" / "muxix"
    if not candidate.exists():
        pytest.skip(f"muxix binary not found at {candidate} — run `cargo build` first")
    return candidate


def run_wm(
    args: list[str],
    *,
    xdg_state: Path,
    extra_env: dict | None = None,
    fake_bin: Path | None = None,
    expect_fail: bool = False,
    timeout: int = 10,
) -> subprocess.CompletedProcess:
    """Run muxix with an isolated state store."""
    env = os.environ.copy()
    env["XDG_STATE_HOME"] = str(xdg_state)
    env["HOME"] = str(xdg_state / "home")
    if fake_bin:
        env["PATH"] = f"{fake_bin}:{env.get('PATH', '')}"
    if extra_env:
        env.update(extra_env)

    result = subprocess.run(
        [str(muxix_exe())] + args,
        env=env,
        capture_output=True,
        text=True,
        timeout=timeout,
        check=False,
    )
    if expect_fail:
        assert result.returncode != 0, (
            f"Expected failure but command succeeded.\nStdout: {result.stdout}"
        )
    else:
        assert result.returncode == 0, (
            f"muxix {' '.join(args)} failed (exit {result.returncode})\n"
            f"Stdout:\n{result.stdout}\nStderr:\n{result.stderr}"
        )
    return result


def seed_agent_state(
    xdg_state: Path,
    *,
    agent_id: str = "aaaabbbb-0000-0000-0000-000000000001",
    pane_id: str = "%42",
    backend: str = "tmux",
    instance: str = "default",
    sandbox_id: str | None = None,
    checkpoint_path: str | None = None,
    checkpoint_ts: int | None = None,
    workdir: str = "/tmp/project",
) -> Path:
    """Write a fake agent state file into the isolated state store."""
    agents_dir = xdg_state / "muxix" / "agents"
    agents_dir.mkdir(parents=True, exist_ok=True)

    # Encode filename the same way PaneKey::to_filename() does
    # (percent-encode / \ : %)
    def encode(s: str) -> str:
        out = []
        for c in s:
            if c in "/:%":
                out.append(f"%{ord(c):02X}")
            else:
                out.append(c)
        return "".join(out)

    filename = f"{backend}__{encode(instance)}__{encode(pane_id)}.json"

    state: dict = {
        "agent_id": agent_id,
        "pane_key": {"backend": backend, "instance": instance, "pane_id": pane_id},
        "workdir": workdir,
        "status": "working",
        "status_ts": int(time.time()),
        "pane_title": None,
        "pane_pid": 12345,
        "command": "opencode",
        "updated_ts": int(time.time()),
        "window_name": "wm-test-feature",
        "session_name": "main",
        "boot_id": None,
        "agent_kind": "opencode",
    }
    if sandbox_id is not None:
        state["sandbox_id"] = sandbox_id
    if checkpoint_path is not None:
        state["checkpoint_path"] = checkpoint_path
    if checkpoint_ts is not None:
        state["checkpoint_ts"] = checkpoint_ts

    path = agents_dir / filename
    path.write_text(json.dumps(state, indent=2))
    return path


def seed_project(
    base: Path,
    *,
    backend: str = "microsandbox",
    checkpoint: dict | None = None,
    runtime: str | None = None,
) -> Path:
    """Create a project directory with a .muxix.yaml selecting the sandbox backend.

    The agent's workdir must point here so that `muxix sandbox checkpoint/resume`
    resolves the sandbox backend from the agent's own project config.

    `runtime` (e.g. "podman" / "docker") forces `sandbox.container.runtime` for
    the container/CRIU backend; otherwise the runtime is auto-detected.
    """
    project = base / "project"
    project.mkdir(parents=True, exist_ok=True)
    sandbox: dict = {"backend": backend}
    if checkpoint is not None:
        sandbox["checkpoint"] = checkpoint
    if runtime is not None:
        sandbox["container"] = {"runtime": runtime}
    (project / ".muxix.yaml").write_text(yaml.safe_dump({"sandbox": sandbox}))
    # muxix resolves project config only inside a git repo, so the agent's
    # worktree must be one. Initialise a throwaway repo.
    subprocess.run(
        ["git", "init", "-q"],
        cwd=project,
        check=True,
        capture_output=True,
    )
    return project


def make_fake_msb(bin_dir: Path, *, log_file: Path, exit_code: int = 0) -> Path:
    """Create a fake `msb` binary that records its argv and exits with exit_code."""
    script = bin_dir / "msb"
    script.write_text(
        "#!/bin/sh\n"
        f"printf '%s\\n' \"$@\" >> {shlex.quote(str(log_file))}\n"
        f"exit {exit_code}\n"
    )
    script.chmod(0o755)
    return script


def read_log(log_file: Path) -> list[str]:
    """Parse the msb call log — each invocation's args on separate lines."""
    if not log_file.exists():
        return []
    return [line for line in log_file.read_text().split("\n") if line]


def make_fake_criu(bin_dir: Path) -> Path:
    """Create a fake `criu` binary so `ensure_criu_available()` succeeds.

    The container CRIU path only checks that `criu` is discoverable on PATH; the
    runtime (podman/docker) is what actually invokes it, so a no-op suffices.
    """
    script = bin_dir / "criu"
    script.write_text("#!/bin/sh\nexit 0\n")
    script.chmod(0o755)
    return script


def make_fake_runtime(
    bin_dir: Path, name: str, *, log_file: Path, exit_code: int = 0
) -> Path:
    """Create a fake container runtime (`podman`/`docker`) that records its argv.

    When invoked with `--export <file>` (podman checkpoint), it touches that file
    so resume's existence check and retention globbing find a real snapshot.
    """
    script = bin_dir / name
    script.write_text(
        "#!/bin/sh\n"
        f"printf '%s\\n' \"$@\" >> {shlex.quote(str(log_file))}\n"
        # Podman writes the archive at --export <path>; create it so the file
        # exists for later resume / retention checks.
        "prev=''\n"
        'for arg in "$@"; do\n'
        '  if [ "$prev" = \'--export\' ]; then touch "$arg"; fi\n'
        "  prev=$arg\n"
        "done\n"
        f"exit {exit_code}\n"
    )
    script.chmod(0o755)
    return script


# ---------------------------------------------------------------------------
# Tests: muxix sandbox checkpoint
# ---------------------------------------------------------------------------


AGENT_ID_1 = "aaaabbbb-1111-0000-0000-000000000001"
AGENT_ID_2 = "aaaabbbb-2222-0000-0000-000000000002"
AGENT_ID_3 = "aaaabbbb-3333-0000-0000-000000000003"


def test_checkpoint_errors_without_sandbox_id(tmp_path):
    """checkpoint command fails gracefully when the agent has no sandbox_id."""
    xdg = tmp_path / "state"
    seed_agent_state(xdg, agent_id=AGENT_ID_1, sandbox_id=None)

    result = run_wm(
        ["sandbox", "checkpoint", AGENT_ID_1],
        xdg_state=xdg,
        expect_fail=True,
    )
    assert "sandbox_id" in result.stderr or "sandbox_id" in result.stdout, (
        f"Expected sandbox_id error message, got stderr={result.stderr!r}"
    )


def test_checkpoint_errors_when_agent_not_found(tmp_path):
    """checkpoint command fails gracefully for an unknown agent ID."""
    xdg = tmp_path / "state"
    (xdg / "muxix" / "agents").mkdir(parents=True, exist_ok=True)

    result = run_wm(
        ["sandbox", "checkpoint", "00000000-dead-beef-0000-000000000000"],
        xdg_state=xdg,
        expect_fail=True,
    )
    assert result.returncode != 0


def test_checkpoint_calls_msb_snapshot(tmp_path):
    """checkpoint command calls `msb sandbox snapshot <vm> --output <path>`."""
    xdg = tmp_path / "state"
    fake_bin = tmp_path / "fake-bin"
    fake_bin.mkdir()
    log = tmp_path / "msb-calls.log"

    make_fake_msb(fake_bin, log_file=log)
    project = seed_project(tmp_path)
    seed_agent_state(
        xdg,
        agent_id=AGENT_ID_1,
        sandbox_id="wm-my-feature-1234",
        workdir=str(project),
    )

    run_wm(
        ["sandbox", "checkpoint", AGENT_ID_1],
        xdg_state=xdg,
        fake_bin=fake_bin,
    )

    args = read_log(log)
    assert "sandbox" in args, f"Expected 'sandbox' subcommand in msb call: {args}"
    assert "snapshot" in args, f"Expected 'snapshot' in msb call: {args}"
    assert "wm-my-feature-1234" in args, (
        f"Expected sandbox_id in msb snapshot call: {args}"
    )
    assert "--output" in args, f"Expected --output flag in msb snapshot call: {args}"


def test_checkpoint_writes_path_to_state(tmp_path):
    """After a successful checkpoint, checkpoint_path is updated in agent state."""
    xdg = tmp_path / "state"
    fake_bin = tmp_path / "fake-bin"
    fake_bin.mkdir()
    log = tmp_path / "msb-calls.log"

    make_fake_msb(fake_bin, log_file=log)
    project = seed_project(tmp_path)
    state_file = seed_agent_state(
        xdg,
        agent_id=AGENT_ID_2,
        sandbox_id="wm-test-5678",
        workdir=str(project),
    )

    run_wm(
        ["sandbox", "checkpoint", AGENT_ID_2],
        xdg_state=xdg,
        fake_bin=fake_bin,
    )

    updated = json.loads(state_file.read_text())
    assert "checkpoint_path" in updated, (
        "checkpoint_path should be written to state after successful checkpoint"
    )
    assert updated["checkpoint_path"] is not None
    assert "checkpoint_ts" in updated and updated["checkpoint_ts"] is not None


# ---------------------------------------------------------------------------
# Tests: muxix sandbox resume
# ---------------------------------------------------------------------------


def test_resume_errors_without_checkpoint_path(tmp_path):
    """resume command fails gracefully when no checkpoint has been recorded."""
    xdg = tmp_path / "state"
    seed_agent_state(xdg, agent_id=AGENT_ID_1, sandbox_id="wm-test-1234")

    result = run_wm(
        ["sandbox", "resume", AGENT_ID_1],
        xdg_state=xdg,
        expect_fail=True,
    )
    assert result.returncode != 0


def test_resume_errors_when_snapshot_missing(tmp_path):
    """resume fails when checkpoint_path is recorded but the file doesn't exist."""
    xdg = tmp_path / "state"
    seed_agent_state(
        xdg,
        agent_id=AGENT_ID_1,
        sandbox_id="wm-test-1234",
        checkpoint_path="/nonexistent/path/snap.snap",
        checkpoint_ts=int(time.time()),
    )

    result = run_wm(
        ["sandbox", "resume", AGENT_ID_1],
        xdg_state=xdg,
        expect_fail=True,
    )
    assert result.returncode != 0


def test_resume_calls_msb_restore(tmp_path):
    """resume calls `msb sandbox restore <vm> --from <snapshot_path>`."""
    xdg = tmp_path / "state"
    fake_bin = tmp_path / "fake-bin"
    fake_bin.mkdir()
    log = tmp_path / "msb-calls.log"

    # Create a real (empty) snapshot file so the existence check passes.
    snap_path = tmp_path / "wm-test-9999-1700000000.snap"
    snap_path.write_bytes(b"")

    # Fake msb records args and exits 0.
    make_fake_msb(fake_bin, log_file=log)

    # Also fake tmux switch-client so the focus call doesn't fail.
    (fake_bin / "tmux").write_text("#!/bin/sh\nexit 0\n")
    (fake_bin / "tmux").chmod(0o755)

    project = seed_project(tmp_path)
    seed_agent_state(
        xdg,
        agent_id=AGENT_ID_1,
        sandbox_id="wm-test-9999",
        checkpoint_path=str(snap_path),
        checkpoint_ts=1700000000,
        workdir=str(project),
    )

    run_wm(
        ["sandbox", "resume", AGENT_ID_1],
        xdg_state=xdg,
        fake_bin=fake_bin,
    )

    args = read_log(log)
    assert "restore" in args, f"Expected 'restore' in msb call: {args}"
    assert "wm-test-9999" in args, f"Expected sandbox_id in msb restore: {args}"
    assert "--from" in args, f"Expected --from flag in msb restore: {args}"
    assert str(snap_path) in args, f"Expected snapshot path in msb restore: {args}"


# ---------------------------------------------------------------------------
# Tests: muxix focus
# ---------------------------------------------------------------------------


def test_focus_errors_for_unknown_target(tmp_path):
    """focus fails with a clear error for an unrecognised agent ID or name."""
    xdg = tmp_path / "state"
    (xdg / "muxix" / "agents").mkdir(parents=True, exist_ok=True)

    result = run_wm(
        ["focus", "00000000-dead-beef-0000-000000000000"],
        xdg_state=xdg,
        expect_fail=True,
    )
    assert result.returncode != 0
    assert (
        "no agent" in result.stderr.lower() or "not found" in result.stderr.lower()
    ), f"Expected 'no agent' error, got: {result.stderr!r}"


def test_focus_by_agent_id_prefix(tmp_path):
    """focus resolves by agent_id prefix (first 8 chars is enough)."""
    xdg = tmp_path / "state"
    fake_bin = tmp_path / "fake-bin"
    fake_bin.mkdir()

    # Fake tmux so the switch call doesn't fail.
    (fake_bin / "tmux").write_text("#!/bin/sh\nexit 0\n")
    (fake_bin / "tmux").chmod(0o755)

    seed_agent_state(xdg, agent_id=AGENT_ID_1)

    # Use just the first 8 chars of the UUID as the prefix.
    run_wm(
        ["focus", AGENT_ID_1[:8]],
        xdg_state=xdg,
        fake_bin=fake_bin,
    )


def test_focus_errors_for_ambiguous_window_name(tmp_path):
    """focus fails when a window-name substring matches multiple agents."""
    xdg = tmp_path / "state"
    seed_agent_state(xdg, agent_id=AGENT_ID_1, pane_id="%10", workdir="/tmp/project-a")
    seed_agent_state(xdg, agent_id=AGENT_ID_2, pane_id="%11", workdir="/tmp/project-b")

    # Patch both to share a common window_name prefix.
    agents_dir = xdg / "muxix" / "agents"
    window_names = ["wm-feature-auth", "wm-feature-auth-v2"]
    for i, f in enumerate(sorted(agents_dir.iterdir())):
        data = json.loads(f.read_text())
        data["window_name"] = window_names[i]
        f.write_text(json.dumps(data))

    result = run_wm(
        ["focus", "wm-feature"],
        xdg_state=xdg,
        expect_fail=True,
    )
    assert result.returncode != 0
    assert (
        "ambiguous" in result.stderr.lower() or "multiple" in result.stderr.lower()
    ), f"Expected ambiguous error, got: {result.stderr!r}"


# ---------------------------------------------------------------------------
# Tests: checkpoint retention (via CLI)
# ---------------------------------------------------------------------------


def test_checkpoint_retention_prunes_old_snapshots(tmp_path):
    """
    With `keep: 1`, a second checkpoint should delete the first snapshot file.
    The fake msb just exits 0; we verify the old file is gone after the
    second checkpoint run.
    """
    xdg = tmp_path / "state"
    fake_bin = tmp_path / "fake-bin"
    fake_bin.mkdir()
    log = tmp_path / "msb-calls.log"

    # Fake msb that also creates the snapshot file so the runner can find it.
    # The snapshot path comes as the arg after --output.
    script = fake_bin / "msb"
    script.write_text(
        "#!/bin/sh\n"
        f"printf '%s\\n' \"$@\" >> {shlex.quote(str(log))}\n"
        # Create the output file so file-existence checks pass.
        "prev=''\n"
        'for arg in "$@"; do\n'
        '  if [ "$prev" = \'--output\' ]; then touch "$arg"; fi\n'
        "  prev=$arg\n"
        "done\n"
        "exit 0\n"
    )
    script.chmod(0o755)

    # Write a .muxix.yaml with microsandbox backend and checkpoint.keep: 1
    project = seed_project(
        tmp_path,
        checkpoint={"enabled": True, "strategy": "manual", "keep": 1},
    )

    seed_agent_state(
        xdg,
        agent_id=AGENT_ID_3,
        sandbox_id="wm-retention-test",
        workdir=str(project),
    )

    # First checkpoint
    run_wm(
        ["sandbox", "checkpoint", AGENT_ID_3],
        xdg_state=xdg,
        fake_bin=fake_bin,
    )

    # Read the checkpoint_path from state after first run
    agents_dir = xdg / "muxix" / "agents"
    state_files = list(agents_dir.iterdir())
    assert state_files, "State file must exist"
    first_state = json.loads(state_files[0].read_text())
    first_snap = Path(first_state.get("checkpoint_path", ""))

    assert first_snap.exists(), f"First snapshot should exist: {first_snap}"

    # Second checkpoint — with keep=1, first snap should be pruned
    run_wm(
        ["sandbox", "checkpoint", AGENT_ID_3],
        xdg_state=xdg,
        fake_bin=fake_bin,
    )

    second_state = json.loads(state_files[0].read_text())
    second_snap = Path(second_state.get("checkpoint_path", ""))

    assert second_snap != first_snap, (
        "Second checkpoint should produce a new snapshot path"
    )
    assert second_snap.exists(), "Second snapshot should exist"
    assert not first_snap.exists(), (
        f"First snapshot should have been pruned (keep=1), but still exists: {first_snap}"
    )


# ---------------------------------------------------------------------------
# Tests: container (Docker/Podman) checkpoint via CRIU
# ---------------------------------------------------------------------------


def test_container_checkpoint_requires_criu(tmp_path):
    """Container checkpoint fails with clear guidance when `criu` is missing."""
    xdg = tmp_path / "state"
    fake_bin = tmp_path / "fake-bin"
    fake_bin.mkdir()
    log = tmp_path / "podman-calls.log"

    # Provide a fake podman but deliberately NO criu.
    make_fake_runtime(fake_bin, "podman", log_file=log)
    project = seed_project(tmp_path, backend="container", runtime="podman")
    seed_agent_state(
        xdg,
        agent_id=AGENT_ID_1,
        sandbox_id="wm-criu-missing",
        workdir=str(project),
    )

    # Restrict PATH to the fake bin (+ git/tar) so a host-installed criu can't
    # accidentally satisfy the availability check.
    aux = {
        os.path.dirname(shutil.which(b))
        for b in ("git", "tar", "sh")
        if shutil.which(b)
    }
    minimal_path = os.pathsep.join([str(fake_bin), *sorted(aux)])
    result = run_wm(
        ["sandbox", "checkpoint", AGENT_ID_1],
        xdg_state=xdg,
        extra_env={"PATH": minimal_path},
        expect_fail=True,
    )
    assert "criu" in result.stderr.lower() or "criu" in result.stdout.lower(), (
        f"Expected CRIU guidance, got stderr={result.stderr!r}"
    )
    # Runtime must not have been invoked when the precondition fails.
    assert not read_log(log), f"podman should not run without criu: {read_log(log)}"


def test_checkpoint_calls_podman_criu(tmp_path):
    """Container/podman checkpoint runs `podman container checkpoint --export <file> <id>`."""
    xdg = tmp_path / "state"
    fake_bin = tmp_path / "fake-bin"
    fake_bin.mkdir()
    log = tmp_path / "podman-calls.log"

    make_fake_criu(fake_bin)
    make_fake_runtime(fake_bin, "podman", log_file=log)
    project = seed_project(tmp_path, backend="container", runtime="podman")
    state_file = seed_agent_state(
        xdg,
        agent_id=AGENT_ID_1,
        sandbox_id="wm-podman-feat",
        workdir=str(project),
    )

    run_wm(
        ["sandbox", "checkpoint", AGENT_ID_1],
        xdg_state=xdg,
        fake_bin=fake_bin,
    )

    args = read_log(log)
    assert "container" in args and "checkpoint" in args, (
        f"Expected `podman container checkpoint`, got: {args}"
    )
    assert "--export" in args, f"Expected --export flag: {args}"
    assert "wm-podman-feat" in args, f"Expected container id in argv: {args}"
    # The export path is the snapshot recorded in state.
    updated = json.loads(state_file.read_text())
    snap = updated.get("checkpoint_path")
    assert snap, "checkpoint_path should be written to state"
    assert snap in args, f"Export path {snap} should be passed to podman: {args}"
    assert updated.get("checkpoint_ts") is not None


def test_resume_calls_podman_criu(tmp_path):
    """Container/podman resume runs `podman container restore --import <file>`."""
    xdg = tmp_path / "state"
    fake_bin = tmp_path / "fake-bin"
    fake_bin.mkdir()
    log = tmp_path / "podman-calls.log"

    snap_path = tmp_path / "wm-podman-feat-1700000000.snap"
    snap_path.write_bytes(b"")

    make_fake_criu(fake_bin)
    make_fake_runtime(fake_bin, "podman", log_file=log)
    # focus path may call tmux after restore.
    (fake_bin / "tmux").write_text("#!/bin/sh\nexit 0\n")
    (fake_bin / "tmux").chmod(0o755)

    project = seed_project(tmp_path, backend="container", runtime="podman")
    seed_agent_state(
        xdg,
        agent_id=AGENT_ID_1,
        sandbox_id="wm-podman-feat",
        checkpoint_path=str(snap_path),
        checkpoint_ts=1700000000,
        workdir=str(project),
    )

    run_wm(
        ["sandbox", "resume", AGENT_ID_1],
        xdg_state=xdg,
        fake_bin=fake_bin,
    )

    args = read_log(log)
    assert "container" in args and "restore" in args, (
        f"Expected `podman container restore`, got: {args}"
    )
    assert "--import" in args, f"Expected --import flag: {args}"
    assert str(snap_path) in args, f"Expected snapshot path in restore argv: {args}"
