import type { Plugin } from '@opencode-ai/plugin';

export const WorkmuxStatusPlugin: Plugin = async ({ $ }) => {
  // OpenCode can emit repeated `session.status busy` events for a single turn,
  // and can even emit a stale trailing `busy` after `idle` at the end. Track
  // per-session status so workmux only sees real transitions.
  const lastStatusBySession = new Map<string, string>();
  const acceptBusyBySession = new Map<string, boolean>();

  async function setStatus(
    sessionID: string | undefined,
    status: string,
  ) {
    if (!sessionID) {
      return;
    }

    const previous = lastStatusBySession.get(sessionID);
    // Ignore the final stale `busy` OpenCode sometimes emits after a session is
    // already done. The next user message re-arms `working` for the new turn.
    if (status === 'working' && acceptBusyBySession.get(sessionID) === false) {
      return;
    }
    if (previous === status) {
      return;
    }

    lastStatusBySession.set(sessionID, status);
    if (status === 'done') {
      acceptBusyBySession.set(sessionID, false);
    } else {
      acceptBusyBySession.set(sessionID, true);
    }

    await $`workmux set-window-status ${status}`.quiet();
    if (status === 'done') {
      // Pane-keyed pipeline signal: lets the harness runner detect turn completion
      // deterministically instead of scraping the pane for an idle prompt.
      try {
        await $`workmux signal turn-done`.quiet();
      } catch {}
    }
  }

  // Emitted once when the plugin loads (≈ session start), so the harness runner's
  // readiness gate has a deterministic "this (re)launched agent is up" signal.
  // Guarded: harmless no-op outside tmux / the pipeline.
  try {
    await $`workmux signal session-ready`.quiet();
  } catch {}

  return {
    event: async ({ event }) => {
      if (event.type === 'message.updated' && event.properties.info.role === 'user') {
        acceptBusyBySession.set(event.properties.sessionID, true);
      }

      switch (event.type) {
        case 'session.status':
          if (event.properties.status.type === 'busy') {
            await setStatus(event.properties.sessionID, 'working');
          }
          if (event.properties.status.type === 'idle') {
            await setStatus(event.properties.sessionID, 'done');
          }
          break;
        case 'permission.asked':
        case 'question.asked':
          await setStatus(event.properties.sessionID, 'waiting');
          break;
        case 'permission.replied':
        case 'question.replied':
          await setStatus(event.properties.sessionID, 'working');
          break;
        case 'session.idle':
          await setStatus(event.properties.sessionID, 'done');
          break;
      }
    },
  };
};
