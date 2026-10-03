#!/usr/bin/env bash
# Launch a detached taskflow run and wait for it to reach a terminal status.
#
#   taskflow-run.sh --define <flow.json> [--launch-timeout 120] [--stall-minutes 90]
#                   [--poll-seconds 30] [--out FILE] [--runs-dir DIR] [--cwd DIR]
#
# Exit codes:
#   0  run completed
#   1  run failed / blocked / paused
#   2  launch_failed (no new run appeared in time) or stalled
#
# The run id is printed on stdout (and written to --out when given); progress
# and diagnostics go to stderr.
#
# The launch itself is one headless agent call (`pi -p` with the taskflow tool).
# That call is an LLM turn and can silently do nothing, so the launch is
# *asserted*: a run whose createdAt is newer than the launch must appear in
# `<runs-dir>/index.json` within --launch-timeout, else this exits 2. Override
# the launch with $TASKFLOW_LAUNCH_CMD (used by the tests; no pi required).
set -uo pipefail

DEFINE=""
LAUNCH_TIMEOUT=120
STALL_MINUTES=90
POLL_SECONDS=30
OUT=""
CWD="$PWD"
RUNS_DIR="${TASKFLOW_RUNS_DIR:-}"

die() { echo "taskflow-run: $*" >&2; exit 2; }

while [ $# -gt 0 ]; do
  case "$1" in
    --define)         DEFINE="${2:-}"; shift 2 ;;
    --launch-timeout) LAUNCH_TIMEOUT="${2:-}"; shift 2 ;;
    --stall-minutes)  STALL_MINUTES="${2:-}"; shift 2 ;;
    --poll-seconds)   POLL_SECONDS="${2:-}"; shift 2 ;;
    --out)            OUT="${2:-}"; shift 2 ;;
    --runs-dir)       RUNS_DIR="${2:-}"; shift 2 ;;
    --cwd)            CWD="${2:-}"; shift 2 ;;
    -h|--help)        sed -n '2,17p' "$0"; exit 0 ;;
    *)                die "unknown argument: $1" ;;
  esac
done

[ -n "$DEFINE" ] || die "--define <flow.json> is required"
[ -f "$DEFINE" ] || die "--define file not found: $DEFINE"
[ -n "$RUNS_DIR" ] || RUNS_DIR="$CWD/.pi/taskflows/runs"
INDEX="$RUNS_DIR/index.json"

now_ms() { python3 -c 'import time; print(int(time.time()*1000))'; }

# Newest run in the index created at/after $1 (and, when the record carries a
# cwd, belonging to $CWD). Prints "<runId>\n<relPath>"; exit 1 when there is none.
newest_run_since() {
  python3 - "$INDEX" "$1" "$CWD" <<'PY'
import json, os, sys
index, since, cwd = sys.argv[1], int(sys.argv[2]), os.path.realpath(sys.argv[3])
try:
    runs = json.load(open(index))
except Exception:
    sys.exit(1)
def mine(r):
    c = r.get("cwd")
    return True if not c else os.path.realpath(c) == cwd
cand = [r for r in runs if isinstance(r, dict) and r.get("createdAt", 0) >= since and mine(r)]
if not cand:
    sys.exit(1)
r = max(cand, key=lambda r: r.get("createdAt", 0))
print(r.get("runId", ""))
print(r.get("relPath", ""))
PY
}

# "<status> <updatedAt>" of a run state file; exit 1 if unreadable.
run_state() {
  python3 - "$1" <<'PY'
import json, sys
try:
    d = json.load(open(sys.argv[1]))
except Exception:
    sys.exit(1)
print(d.get("status", ""), int(d.get("updatedAt", 0)))
PY
}

# Cancel marker path for (cwd, runId), mirroring taskflow's detachedCancelRequestPath:
# sha256(realpath \0 st_dev \0 st_ino) under <agent dir>/taskflow-control/.
cancel_request_path() {
  python3 - "$CWD" "$1" <<'PY'
import hashlib, os, sys
cwd = os.path.realpath(sys.argv[1])
st = os.stat(cwd)
key = hashlib.sha256(f"{cwd}\0{st.st_dev}\0{st.st_ino}".encode()).hexdigest()
agent = (os.environ.get("TASKFLOW_AGENT_DIR") or os.environ.get("PI_CODING_AGENT_DIR")
         or os.path.join(os.path.expanduser("~"), ".pi", "agent"))
d = os.path.join(os.path.expanduser(agent), "taskflow-control", key)
os.makedirs(d, mode=0o700, exist_ok=True)
print(os.path.join(d, sys.argv[2] + ".cancel.json"))
PY
}

LAUNCHED_AT="$(now_ms)"

if [ -n "${TASKFLOW_LAUNCH_CMD:-}" ]; then
  DEFINE="$DEFINE" CWD="$CWD" RUNS_DIR="$RUNS_DIR" bash -c "$TASKFLOW_LAUNCH_CMD" >&2
else
  "${PI_BIN:-pi}" --no-session -p "Call the taskflow tool EXACTLY ONCE with {\"action\":\"run\",\"defineFile\":\"$DEFINE\",\"detach\":true}, report the runId it returns, then stop. Do not read files, do not plan, do not call any other tool." >&2
fi
echo "taskflow-run: launch call returned (exit $?), asserting a run appeared" >&2

RUN_ID=""
REL_PATH=""
deadline=$(( $(date +%s) + LAUNCH_TIMEOUT ))
while :; do
  if found="$(newest_run_since "$LAUNCHED_AT")"; then
    RUN_ID="$(printf '%s\n' "$found" | sed -n 1p)"
    REL_PATH="$(printf '%s\n' "$found" | sed -n 2p)"
    [ -n "$RUN_ID" ] && break
  fi
  [ "$(date +%s)" -lt "$deadline" ] || {
    echo "taskflow-run: launch_failed — no run created within ${LAUNCH_TIMEOUT}s (index: $INDEX)" >&2
    exit 2
  }
  sleep 2
done

echo "$RUN_ID"
[ -n "$OUT" ] && printf '%s\n' "$RUN_ID" > "$OUT"
echo "taskflow-run: run $RUN_ID launched" >&2

RUN_JSON="$RUNS_DIR/$REL_PATH"
[ -n "$REL_PATH" ] || RUN_JSON="$(find "$RUNS_DIR" -name "$RUN_ID.json" -print -quit 2>/dev/null)"
STALL_MS=$(( STALL_MINUTES * 60 * 1000 ))

while :; do
  if state="$(run_state "$RUN_JSON")"; then
    status="${state%% *}"
    updated="${state##* }"
    case "$status" in
      running|"") ;;
      completed) echo "taskflow-run: $RUN_ID completed" >&2; exit 0 ;;
      *)         echo "taskflow-run: $RUN_ID $status" >&2; exit 1 ;;
    esac
    if [ "$updated" -gt 0 ] && [ $(( $(now_ms) - updated )) -gt "$STALL_MS" ]; then
      marker="$(cancel_request_path "$RUN_ID")"
      printf '{"requestedAt":%s,"reason":"muxix: stalled past %s minutes"}\n' \
        "$(now_ms)" "$STALL_MINUTES" > "$marker"
      echo "taskflow-run: $RUN_ID stalled (no update for >${STALL_MINUTES}m), cancel requested at $marker" >&2
      exit 2
    fi
  fi
  sleep "$POLL_SECONDS"
done
