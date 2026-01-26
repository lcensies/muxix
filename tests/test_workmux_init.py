import os
import subprocess
from pathlib import Path

import pytest
import yaml

from .conftest import MuxEnvironment


def test_init_creates_config_file_on_success(
    mux_server: MuxEnvironment, workmux_exe_path: Path
):
    """Verifies `workmux init` creates .workmux.yaml with correct content."""
    env = mux_server
    config_file_path = env.tmp_path / ".workmux.yaml"

    assert not config_file_path.exists()

    result = env.run_command([str(workmux_exe_path), "init"])

    assert result.returncode == 0
    assert config_file_path.is_file(), "The .workmux.yaml file was not created"
    assert "✓ Created .workmux.yaml" in result.stdout
    assert "For global settings, edit" in result.stdout

    content = config_file_path.read_text()
    try:
        yaml.safe_load(content)
    except yaml.YAMLError as e:
        pytest.fail(f"Generated .workmux.yaml is not valid YAML: {e}")

    assert "# workmux project configuration" in content
    assert "symlink:" in content
    assert '"<global>"' in content
    assert "post_create:" in content


def test_init_fails_if_config_exists(
    mux_server: MuxEnvironment, workmux_exe_path: Path
):
    """Verifies `workmux init` fails if .workmux.yaml already exists."""
    env = mux_server
    config_file_path = env.tmp_path / ".workmux.yaml"
    preexisting_content = "pre-existing content"

    config_file_path.write_text(preexisting_content)

    with pytest.raises(subprocess.CalledProcessError) as exc_info:
        env.run_command([str(workmux_exe_path), "init"])

    assert exc_info.value.returncode != 0
    assert ".workmux.yaml already exists" in exc_info.value.stderr
    assert config_file_path.read_text() == preexisting_content


def test_init_fails_in_readonly_directory(
    mux_server: MuxEnvironment, workmux_exe_path: Path
):
    """Verifies `workmux init` fails gracefully if the directory is not writable."""
    env = mux_server
    test_dir = env.tmp_path
    config_file_path = test_dir / ".workmux.yaml"

    original_mode = test_dir.stat().st_mode
    os.chmod(test_dir, 0o555)

    try:
        with pytest.raises(subprocess.CalledProcessError) as exc_info:
            env.run_command([str(workmux_exe_path), "init"])

        assert exc_info.value.returncode != 0
        assert (
            "permission" in exc_info.value.stderr.lower()
            or "os error 13" in exc_info.value.stderr.lower()
        )
        assert not config_file_path.exists()

    finally:
        os.chmod(test_dir, original_mode)
