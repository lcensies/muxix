---
description: Daemon plus client architecture behind the sidebar
---

# How it works

The sidebar is a bit of a hack on top of tmux's pane system, but it works quite
well. It uses a daemon + client architecture with event-driven rendering:

1. **Toggle on** (`workmux sidebar`): creates a narrow tmux pane on the left
   side of every window using a full-height split, starts a background daemon,
   and installs tmux hooks.

2. **Daemon**: a single headless process that polls tmux state every 2 seconds
   (or immediately when signaled via SIGUSR1). It reads agent state from the
   filesystem, queries tmux for pane geometry and active windows, then pushes
   snapshots to all connected sidebar clients over a Unix socket.

3. **Clients**: every tmux window gets its own sidebar pane running a separate
   `workmux _sidebar-run` process. Each process connects to the shared daemon
   socket, receives snapshots via a background reader thread, and renders
   independently. The main thread blocks on a channel, only waking when new
   data arrives or a spinner tick is needed. Rendering is skipped entirely for
   inactive windows. This event-driven design keeps CPU usage near zero when
   idle.

4. **Hooks**: tmux hooks handle lifecycle events:
   - `after-new-window` / `after-new-session`: automatically adds a sidebar pane
     to newly created windows
   - `window-resized`: reflows the layout tree to keep the sidebar at the
     correct width and content panes proportionally balanced
   - `after-select-window` / `client-session-changed` / `after-kill-pane`:
     signals the daemon for an immediate refresh

5. **Layout reflow**: when the sidebar is added or the terminal is resized, a
   layout tree parser reads the tmux `#{window_layout}` string, scales the
   content subtree proportionally, and applies the result atomically via
   `select-layout`. This preserves existing pane proportions (e.g. a 70/30 split
   stays 70/30).

6. **Toggle off**: kills all sidebar panes, reflows content panes to fill the
   freed space, stops the daemon, and removes hooks.

## Resource usage

Because tmux has no concept of a pane that persists across all windows, each
window runs its own `_sidebar-run` process. Each one uses roughly 15 MB of
resident memory, and the shared daemon (`_sidebar-daemon`) uses about 16 MB. With
five agents running, total memory footprint is around 90 MB. CPU usage is near
zero when idle thanks to the event-driven architecture.
