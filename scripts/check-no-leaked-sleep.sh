#!/usr/bin/env bash
# Fail if a unit test leaked its `sleep 9999` fixture child.
#
# Such a process inherits the test runner's stdout, so after cargo exits
# anything reading that pipe to EOF -- a shell pipeline, CI, an agent
# capturing output -- blocks forever. `ShellWithChild` in
# src/command/sidebar/daemon.rs kills its process group on Drop; this is the
# regression check that it keeps working.
#
# `-x` matches the whole command line, so a shell that merely mentions the
# pattern in its own argv is not a hit. An unrelated, genuine `sleep 9999` of
# the same user still is.
set -uo pipefail

leaked=$(pgrep -u "$(id -u)" -x -f 'sleep 9999' | tr '\n' ' ')
if [ -n "${leaked// /}" ]; then
    echo "error: leaked test fixture processes (sleep 9999): $leaked" >&2
    # shellcheck disable=SC2086
    ps -o pid,ppid,stat,command -p $leaked >&2 || true
    exit 1
fi
