"""
REAL end-to-end checkpoint/restore test for the container (CRIU) backend.

Unlike `test_sandbox_checkpoint.py` (which uses fake `msb`/`podman`/`criu` and
only asserts the *commands* muxix builds), this test drives the actual
`muxix sandbox checkpoint` / `muxix sandbox resume` binary against a REAL
running container and a REAL CRIU checkpoint, then proves the agent process was
genuinely RESUMED — not restarted:

  1. same in-memory nonce survives the round-trip  → same process
  2. its tick counter keeps advancing after restore → still executing / live
  3. input fed AFTER restore is reflected in output → "you can still type and
     the pane content changes"

It is opt-in and self-skipping. It needs a runtime that can actually CRIU-
checkpoint a container. Rootless podman refuses ("checkpointing a container
requires root"), so this drives the muxix binary under `sudo` with `criu` on
PATH. Run it with:

    MUXIX_CRIU_E2E=1 nix-shell -p criu --run \
      'tests/venv/bin/python -m pytest tests/test_sandbox_checkpoint_e2e.py -v -s'

A plain agent (opencode/claude) is deliberately NOT used as the in-container
process: it would require API keys, network and a model, making the test non-
hermetic and slow. A tiny stateful shell loop is a faithful stand-in — it proves
the exact property that matters (the *same* process resumes and still consumes
input), which is independent of which agent runs inside.
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

IMAGE = "alpine:latest"
SANDBOX_ID = "wm-e2e-criu"
AGENT_ID = "aaaabbbb-e2e0-0000-0000-0000000000e2"

# A stateful, interactive process: fixes an in-memory nonce at boot, advances a
# tick counter several times a second, and echoes whatever was last written to
# /input. The fast tick lets the test poll for state changes instead of sleeping
# on fixed timers, so the round-trip is bound by CRIU latency, not wall clocks.
CONTAINER_CMD = (
    'n="boot-$(date +%s%N)-$$"; echo "$n" > /nonce; i=0; : > /input; '
    'while true; do i=$((i+1)); t="$(cat /input 2>/dev/null)"; '
    'printf "%s tick=%s typed=%s\\n" "$n" "$i" "$t" > /output; sleep 0.1; done'
)


# ---------------------------------------------------------------------------
# Prerequisites / gating
# ---------------------------------------------------------------------------


def _have_passwordless_sudo() -> bool:
    try:
        return subprocess.run(["sudo", "-n", "true"], capture_output=True).returncode == 0
    except Exception:
        return False


def _require_e2e() -> str:
    """Skip unless explicitly enabled and all prerequisites are present.

    Returns the directory containing the `criu` binary (to put on PATH for the
    privileged muxix invocations)."""
    if os.environ.get("MUXIX_CRIU_E2E") != "1":
        pytest.skip("set MUXIX_CRIU_E2E=1 to run the real CRIU e2e test")
    criu = shutil.which("criu")
    if not criu:
        pytest.skip("criu not found on PATH (try: nix-shell -p criu)")
    if not shutil.which("podman"):
        pytest.skip("podman not found on PATH")
    if not _have_passwordless_sudo():
        pytest.skip("passwordless sudo required (rootless podman cannot CRIU-checkpoint)")
    return str(Path(criu).parent)


def muxix_exe() -> Path:
    candidate = Path(__file__).parent.parent / "target" / "debug" / "muxix"
    if not candidate.exists():
        pytest.skip(f"muxix binary not found at {candidate} — run `cargo build` first")
    return candidate


# ---------------------------------------------------------------------------
# Privileged command helpers (rootful podman + criu on PATH)
# ---------------------------------------------------------------------------


def _sudo_env(criu_dir: str, *args: str, **env: str) -> list[str]:
    """`sudo env PATH=<criu>:<PATH> KEY=VAL ... <args>`."""
    path = f"{criu_dir}:{os.environ.get('PATH', '')}"
    kv = [f"PATH={path}"] + [f"{k}={v}" for k, v in env.items()]
    return ["sudo", "env", *kv, *args]


def _podman(criu_dir: str, *args: str, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(
        _sudo_env(criu_dir, "podman", *args),
        capture_output=True,
        text=True,
        check=check,
        timeout=120,
    )


def _rm(criu_dir: str, *, check: bool = True) -> subprocess.CompletedProcess:
    # `-t 0`: SIGKILL immediately. Our container's busybox `sh` loop ignores
    # SIGTERM, so a default `rm -f` would block on the full 10s stop-grace.
    return _podman(criu_dir, "rm", "-f", "-t", "0", SANDBOX_ID, check=check)


def _container_output(criu_dir: str) -> str:
    return _podman(criu_dir, "exec", SANDBOX_ID, "cat", "/output").stdout.strip()


def _wait_output(criu_dir: str, predicate, *, timeout: float = 15.0) -> str:
    """Poll /output until `predicate(line)` is true (or timeout). Tolerates the
    container not being exec-ready yet (e.g. right after restore)."""
    deadline = time.monotonic() + timeout
    last = ""
    while time.monotonic() < deadline:
        r = _podman(criu_dir, "exec", SANDBOX_ID, "cat", "/output", check=False)
        if r.returncode == 0:
            last = r.stdout.strip()
            if predicate(last):
                return last
        time.sleep(0.1)
    return last


def _parse(line: str) -> dict:
    """Parse 'boot-<...> tick=<n> typed=<...>' into a dict."""
    out: dict = {"nonce": line.split(" tick=")[0]}
    if "tick=" in line:
        rest = line.split("tick=", 1)[1]
        out["tick"] = int(rest.split(" ", 1)[0])
    if "typed=" in line:
        out["typed"] = line.split("typed=", 1)[1]
    return out


# ---------------------------------------------------------------------------
# State / project seeding (mirrors test_sandbox_checkpoint.py helpers)
# ---------------------------------------------------------------------------


def _seed_project(base: Path, *, keep: int = 2) -> Path:
    project = base / "project"
    project.mkdir(parents=True, exist_ok=True)
    cfg = {
        "sandbox": {
            "backend": "container",
            "container": {"runtime": "podman"},
            "checkpoint": {"enabled": True, "strategy": "manual", "keep": keep},
        }
    }
    (project / ".muxix.yaml").write_text(yaml.safe_dump(cfg))
    subprocess.run(["git", "init", "-q"], cwd=project, check=True, capture_output=True)
    return project


def _seed_agent_state(xdg: Path, *, workdir: str) -> Path:
    agents_dir = xdg / "muxix" / "agents"
    agents_dir.mkdir(parents=True, exist_ok=True)

    backend, instance, pane_id = "tmux", "default", "%42"

    def encode(s: str) -> str:
        return "".join(f"%{ord(c):02X}" if c in "/:%" else c for c in s)

    filename = f"{backend}__{encode(instance)}__{encode(pane_id)}.json"
    state = {
        "agent_id": AGENT_ID,
        "pane_key": {"backend": backend, "instance": instance, "pane_id": pane_id},
        "workdir": workdir,
        "status": "working",
        "status_ts": int(time.time()),
        "pane_title": None,
        "pane_pid": 12345,
        "command": "opencode",
        "updated_ts": int(time.time()),
        "window_name": "wm-e2e-feature",
        "session_name": "main",
        "boot_id": None,
        "agent_kind": "opencode",
        "sandbox_id": SANDBOX_ID,
    }
    path = agents_dir / filename
    path.write_text(json.dumps(state, indent=2))
    return path


def _read_state(state_file: Path) -> dict:
    # muxix runs as root and rewrites the state file (root-owned), so read it
    # back with sudo to avoid permission surprises.
    raw = subprocess.run(
        ["sudo", "cat", str(state_file)], capture_output=True, text=True, check=True
    ).stdout
    return json.loads(raw)


# ---------------------------------------------------------------------------
# The test
# ---------------------------------------------------------------------------


def test_container_checkpoint_resume_resumes_process(tmp_path):
    criu_dir = _require_e2e()
    wm = muxix_exe()

    xdg = tmp_path / "state"
    home = tmp_path / "home"
    home.mkdir(parents=True, exist_ok=True)
    project = _seed_project(tmp_path)
    state_file = _seed_agent_state(xdg, workdir=str(project))

    snap_path_holder: dict = {}

    def muxix(sub: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            _sudo_env(
                criu_dir,
                str(wm),
                "sandbox",
                sub,
                AGENT_ID,
                HOME=str(home),
                XDG_STATE_HOME=str(xdg),
            ),
            capture_output=True,
            text=True,
            timeout=180,
        )

    try:
        # 1. Launch the real stateful container under the seeded sandbox_id.
        _rm(criu_dir, check=False)
        _podman(
            criu_dir, "run", "-d",
            "--name", SANDBOX_ID, IMAGE, "sh", "-c", CONTAINER_CMD,
        )
        before = _parse(_wait_output(criu_dir, lambda s: "tick=" in s))
        assert "tick" in before, f"container did not produce output: {before}"

        # 2. Checkpoint via the actual muxix binary.
        r = muxix("checkpoint")
        assert r.returncode == 0, f"checkpoint failed:\nstdout={r.stdout}\nstderr={r.stderr}"
        assert "Checkpoint saved" in r.stdout, r.stdout

        st = _read_state(state_file)
        snap = st.get("checkpoint_path")
        assert snap, f"checkpoint_path not written to state: {st}"
        snap_path_holder["snap"] = snap
        assert (
            subprocess.run(["sudo", "test", "-f", snap]).returncode == 0
        ), f"snapshot archive missing on disk: {snap}"
        assert st.get("checkpoint_ts") is not None

        # 3. "Stop" the container (frees memory / simulates pane swap-out).
        _rm(criu_dir)
        assert (
            SANDBOX_ID
            not in _podman(criu_dir, "ps", "-a", "--format", "{{.Names}}").stdout
        )

        # 4. Resume via the actual muxix binary (tmux focus warning is OK).
        r = muxix("resume")
        assert r.returncode == 0, f"resume failed:\nstdout={r.stdout}\nstderr={r.stderr}"

        # 5. Container is back; wait for it to be live, feed NEW input, and let
        # the loop pick it up (poll instead of sleeping on a fixed timer).
        _wait_output(criu_dir, lambda s: "tick=" in s)
        token = "TYPED-AFTER-RESUME-42"
        _podman(criu_dir, "exec", SANDBOX_ID, "sh", "-c", f"echo {token} > /input")
        after = _parse(_wait_output(criu_dir, lambda s: f"typed={token}" in s))

        # 6. Prove RESUME, not restart.
        assert after["nonce"] == before["nonce"], (
            f"nonce changed → process was restarted, not resumed: "
            f"{before['nonce']!r} -> {after['nonce']!r}"
        )
        assert after["tick"] > before["tick"], (
            f"counter did not advance → process not live after restore: "
            f"{before['tick']} -> {after['tick']}"
        )
        assert after.get("typed") == token, (
            f"input fed after restore was not processed (can't 'type' into it): {after}"
        )
    finally:
        _rm(criu_dir, check=False)
        snap = snap_path_holder.get("snap")
        if snap:
            subprocess.run(["sudo", "rm", "-f", snap], check=False)
        # state dir is rewritten root-owned by the privileged muxix run.
        subprocess.run(["sudo", "rm", "-rf", str(xdg)], check=False)
