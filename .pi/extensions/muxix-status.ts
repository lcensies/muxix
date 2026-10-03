/**
 * Muxix status tracking extension for pi.
 *
 * Reports agent status to muxix for tmux window status display.
 * See: https://muxix.dev/guide/status-tracking
 *
 * Also handles prompt injection. The inject file is written by
 * `muxix bootstrap` to `~/.pi/agent/muxix-pre-inject.md`.
 *
 * Injection uses the `context` hook to append the content to the first user
 * message, NOT `before_agent_start`'s `systemPrompt`. Two reasons the system
 * prompt route is unreliable: (1) cliproxy (e.g. the Anthropic OAuth path)
 * forces its own canonical system prompt and discards what pi sends, and
 * (2) pi does not accumulate `systemPrompt` across multiple extensions'
 * `before_agent_start` hooks, so a later extension clobbers our return. The
 * conversation messages are part of the request payload and survive both.
 */

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import * as fs from "node:fs";
import * as path from "node:path";
import * as os from "node:os";

const INJECT_MARKER = "<muxix-inject>";

/** Longest title worth sending: the sidebar's third line clips beyond this. */
const TITLE_MAX = 40;

/**
 * Flatten a user message's content into a one-line pane title.
 *
 * Cuts at INJECT_MARKER so our own injected block never becomes the title —
 * on a single-turn session the last user message IS the injected one.
 * Returns "" when nothing usable is left, meaning: leave the title alone.
 */
export function titleFromContent(content: unknown): string {
  const raw =
    typeof content === "string"
      ? content
      : Array.isArray(content)
        ? content
            .map((b: any) => (typeof b?.text === "string" ? b.text : ""))
            .join(" ")
        : "";

  const text = raw.split(INJECT_MARKER)[0];
  // eslint-disable-next-line no-control-regex
  const flat = text.replace(/[\u0000-\u001f\u007f]/g, " ").replace(/\s+/g, " ").trim();
  return flat.length > TITLE_MAX ? flat.slice(0, TITLE_MAX - 1).trimEnd() + "\u2026" : flat;
}

/** Agent-state JSON files live here: `$XDG_STATE_HOME/muxix/agents` (default `~/.local/state/muxix/agents`). */
export function agentStateDir(): string {
  const stateHome = process.env.XDG_STATE_HOME;
  const base =
    stateHome && path.isAbsolute(stateHome) ? stateHome : path.join(os.homedir(), ".local", "state");
  return path.join(base, "muxix", "agents");
}

/**
 * Walk up from `startDir` looking for a `.git` entry.
 *
 * A `.git` FILE marks a linked worktree checkout (its handle is the basename
 * of the directory that contains it). A `.git` DIRECTORY marks the main
 * worktree/repo root — stop there without naming, since climbing further up
 * would leave the repo entirely.
 */
export function findWorktreeHandle(startDir: string): string | null {
  let dir = startDir;
  for (;;) {
    try {
      const st = fs.statSync(path.join(dir, ".git"));
      if (st.isFile()) return path.basename(dir);
      if (st.isDirectory()) return null;
    } catch {
      // no .git here, keep climbing
    }
    const parent = path.dirname(dir);
    if (parent === dir) return null;
    dir = parent;
  }
}

export default function (pi: ExtensionAPI) {
  function setStatus(status: string) {
    pi.exec("muxix", ["set-window-status", status]).catch(() => {});
  }

  // Fail-open: any watcher/handle-resolution error is logged once and
  // otherwise swallowed so a bug here never breaks the host session.
  let warned = false;
  function warnOnce(err: unknown) {
    if (warned) return;
    warned = true;
    console.warn("[muxix-status]", err);
  }

  // Set once agent_start fires, cleared on agent_settled. Distinguishes a
  // real UI prompt mid-turn (ui_prompt_* while running) from one that fires
  // outside a turn (e.g. a command), which should not clobber pane status.
  let running = false;

  // This session's own worktree handle, resolved at session_start.
  let ownHandle: string | null = null;

  // Child-worktree completion/waiting watcher (§6.4).
  let fsWatcher: fs.FSWatcher | null = null;
  let pollTimer: ReturnType<typeof setInterval> | null = null;
  const lastSeen = new Map<string, { status: unknown; completionTs: unknown }>();

  async function getChildHandles(handle: string): Promise<string[]> {
    const result = await pi.exec("muxix", ["project-state", "show"]).catch(() => null);
    if (!result || result.code !== 0) return [];
    try {
      const doc = JSON.parse(result.stdout);
      const worktrees = doc?.worktrees ?? {};
      return Object.entries(worktrees as Record<string, any>)
        .filter(([, rec]) => rec?.parent === handle)
        .map(([h]) => h);
    } catch {
      return [];
    }
  }

  function tickWatcher(handle: string) {
    getChildHandles(handle)
      .then((children) => {
        if (children.length === 0) return;
        const dir = agentStateDir();
        let files: string[];
        try {
          files = fs.readdirSync(dir).filter((f) => f.endsWith(".json"));
        } catch {
          return;
        }
        for (const file of files) {
          let rec: any;
          try {
            rec = JSON.parse(fs.readFileSync(path.join(dir, file), "utf8"));
          } catch {
            continue;
          }
          const workdir = rec?.workdir;
          if (typeof workdir !== "string") continue;
          const childHandle = path.basename(workdir);
          if (!children.includes(childHandle)) continue;

          const status = rec.status;
          const completionTs = rec.completion?.ts;
          const prev = lastSeen.get(childHandle);
          const unchanged =
            prev !== undefined && prev.status === status && prev.completionTs === completionTs;
          if (unchanged) continue;

          const completionChanged = rec.completion && (!prev || prev.completionTs !== completionTs);
          const startedWaiting = status === "waiting" && (!prev || prev.status !== "waiting");

          if (completionChanged) {
            const kind = rec.completion.kind ?? "completed";
            const feedback = rec.completion.feedback ?? "";
            pi
              .sendMessage(
                {
                  customType: "muxix-completion",
                  content: `${childHandle} ${kind}: ${feedback}`,
                  display: true,
                },
                { deliverAs: "followUp", triggerTurn: true },
              )
              .catch(() => {});
          } else if (startedWaiting) {
            pi
              .sendMessage(
                {
                  customType: "muxix-waiting",
                  content: `${childHandle} is waiting for input`,
                  display: true,
                },
                { deliverAs: "followUp", triggerTurn: true },
              )
              .catch(() => {});
          }

          lastSeen.set(childHandle, { status, completionTs });
        }
      })
      .catch(warnOnce);
  }

  function stopWatcher() {
    if (fsWatcher) {
      try {
        fsWatcher.close();
      } catch {}
      fsWatcher = null;
    }
    if (pollTimer) {
      clearInterval(pollTimer);
      pollTimer = null;
    }
    lastSeen.clear();
  }

  // Always armed inside a worktree: children appear after session_start (the
  // coordinator spawns them), so eligibility is decided per tick, not here.
  function startWatcher(handle: string) {
    try {
      const dir = agentStateDir();
      try {
        fsWatcher = fs.watch(dir, { persistent: false }, () => tickWatcher(handle));
      } catch (err) {
        warnOnce(err);
      }
      pollTimer = setInterval(() => tickWatcher(handle), 2000);
      tickWatcher(handle);
    } catch (err) {
      warnOnce(err);
    }
  }

  // Pane-keyed pipeline signal emitted once at extension load (≈ session start),
  // so the harness runner's readiness gate has a deterministic "this (re)launched
  // agent is up" signal. `.catch` makes it a harmless no-op outside the pipeline.
  pi.exec("muxix", ["signal", "session-ready"]).catch(() => {});

  // Resolve this session's worktree handle and, if this worktree spawned any
  // child worktrees, watch for their completion/waiting transitions so this
  // (parent) session gets nudged when a child needs attention.
  pi.on("session_start", async (_event: any, ctx: any) => {
    try {
      ownHandle = findWorktreeHandle(ctx.cwd);
      if (ownHandle) {
        pi.setSessionName?.(ownHandle);
        startWatcher(ownHandle);
      }
    } catch (err) {
      warnOnce(err);
    }
  });

  pi.on("session_shutdown", async () => {
    stopWatcher();
  });

  // Inject the muxix prompt by appending it to the first user message.
  // Reads ~/.pi/agent/muxix-pre-inject.md (written by `muxix bootstrap`).
  // No-op if the file is absent/empty or the content is already present.
  pi.on("context", async (event: any, ctx: any) => {
    const messages = (event.messages as any[]).slice();

    // Line 3 of a muxix sidebar row is the pane title, and for pi that is
    // still the shell's command title unless something claims the channel.
    //
    // A session name is the better title when one exists: /name, or a titling
    // extension (pi-session-title, pi-session-name) that asked a model for a
    // real summary. Those only call setSessionName, which does not refresh the
    // terminal title (earendil-works/pi#3686), so this is the bridge rather
    // than a competing source. Without one, fall back to the user's own words.
    //
    // Read before injection so the title never reports what muxix appended.
    // Fire-and-forget — a title must never break a turn.
    const named = (() => {
      try {
        return pi.getSessionName?.() ?? "";
      } catch {
        return "";
      }
    })();
    const newest = [...messages].reverse().find((m) => m?.role === "user");
    const title = titleFromContent(named || newest?.content);
    if (title) {
      try {
        ctx?.ui?.setTitle(title);
      } catch {}
    }

    const home = process.env.HOME;
    const agentDir =
      process.env.OMP_CODING_AGENT_DIR ??
      process.env.PI_CODING_AGENT_DIR ??
      (home ? home + "/.pi/agent" : null);
    if (!agentDir) return {};

    const injectPath = agentDir + "/muxix-pre-inject.md";
    const result = await pi.exec("cat", [injectPath]).catch(() => null);
    if (!result || result.code !== 0 || !result.stdout.trim()) return {};

    const idx = messages.findIndex((m) => m?.role === "user");
    if (idx < 0) return {};

    const block = `\n\n${INJECT_MARKER}\n${result.stdout.trim()}\n</muxix-inject>`;
    const hasMarker = (s: unknown) =>
      typeof s === "string" && s.includes(INJECT_MARKER);
    const target = messages[idx];
    const content = target.content;

    if (typeof content === "string") {
      if (hasMarker(content)) return {}; // idempotent
      messages[idx] = { ...target, content: content + block };
    } else if (Array.isArray(content)) {
      if (content.some((b: any) => hasMarker(b?.text))) return {}; // idempotent
      messages[idx] = {
        ...target,
        content: [...content, { type: "text", text: block }],
      };
    } else {
      return {};
    }

    return { messages };
  });

  pi.on("agent_start", async () => {
    running = true;
    setStatus("working");
  });

  // agent_end may still be followed by auto-retry/compaction/queued follow-ups;
  // agent_settled is the point pi is guaranteed not to continue on its own, so
  // it is the correct place for the pane's terminal "done" status + signal.
  pi.on("agent_settled", async () => {
    running = false;
    setStatus("done");
    // Deterministic turn completion for the harness runner (vs content scraping).
    pi.exec("muxix", ["signal", "turn-done"]).catch(() => {});
  });

  // Blocking extension UI prompts (ctx.ui.select/confirm/input/editor/custom)
  // mid-turn mean pi is waiting on the human, not crunching — reflect that in
  // the pane status and restore "working" once the prompt closes. Ignore
  // prompts that fire outside a turn (running === false) so they don't
  // clobber an already-settled "done" status.
  pi.on("ui_prompt_start", async () => {
    if (running) setStatus("waiting");
  });

  pi.on("ui_prompt_end", async () => {
    if (running) setStatus("working");
  });
}
