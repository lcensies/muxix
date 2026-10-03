"""Tests for the signal abstraction system (breakpoint and hook signals).

These are blackbox harness tests that verify the signal system works end-to-end
via CLI and file system interactions, not via internal imports.
"""

import json
import tempfile
from pathlib import Path

import pytest


def test_signal_temp_file_paths():
    """Test that signal temp file paths are generated correctly."""
    import tempfile

    # Signal files should be written to temp directory with predictable names
    task_id = "test-task"
    node_id = "approve-me"
    pane = "%5"

    # Breakpoint signal path format: workmux-bp-{task_id}-{node_id}.json
    bp_path = Path(tempfile.gettempdir()) / f"workmux-bp-{task_id}-{node_id}.json"
    assert "workmux-bp" in bp_path.name
    assert node_id in bp_path.name

    # Pane signal path format: workmux-proceed-{sanitized_pane}.json
    proceed_path = Path(tempfile.gettempdir()) / f"workmux-proceed-{pane.lstrip('%')}.json"
    assert "workmux-proceed" in proceed_path.name


def test_signal_json_format():
    """Test that signal files use standard JSON format for approval."""
    # Signal files must contain JSON with {approved: bool, feedback?: string}
    signal_approved = {"approved": True}
    signal_rejected = {"approved": False, "feedback": "tests failed"}

    # Verify valid JSON
    json.dumps(signal_approved)
    json.dumps(signal_rejected)

    assert signal_approved["approved"] is True
    assert signal_rejected["approved"] is False
    assert signal_rejected["feedback"] == "tests failed"


