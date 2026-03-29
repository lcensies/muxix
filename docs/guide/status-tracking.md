---
description: Display agent status in your tmux window list for at-a-glance visibility
---

# Status tracking

Workmux can display the status of the agent in your tmux window list, giving you at-a-glance visibility into what the agent in each window is doing.

<div style="display: flex; justify-content: center; margin: 1.5rem 0;">
  <img src="/status.webp" alt="tmux status showing agent icons" style="border-radius: 4px;">
</div>

## Agent support

| Agent        | Status                                                                      |
| ------------ | --------------------------------------------------------------------------- |
| Claude Code  | ✅ Supported                                                                |
| OpenCode     | ✅ Supported                                                                |
| Codex        | ✅ Supported\*                                                              |
| Copilot CLI  | ✅ Supported\*                                                              |
| Pi           | ✅ Supported\*                                                              |
| Gemini CLI   | [In progress](https://github.com/google-gemini/gemini-cli/issues/9070)      |
| Kiro         | [Tracking issue](https://github.com/kirodotdev/Kiro/issues/5440)            |
| Mistral Vibe | [Tracking issue](https://github.com/mistralai/mistral-vibe/discussions/334) |

**Notes:**

- **Codex**: No 💬 waiting state. Requires `codex_hooks = true` in `~/.codex/config.toml` (see [Codex setup](#codex-setup))
- **Copilot CLI**: No 💬 waiting state
- **Pi**: No 💬 waiting state
- **Kiro**: Hooks support is messy: requires a custom agent since the default can't be edited

## Status icons

- 🤖 = agent is working
- 💬 = agent is waiting for user input
- ✅ = agent finished (auto-clears on window focus)
- 🛑 = agent appears interrupted (no output for 10s)

## Automated setup

Run `workmux setup` to automatically detect your agent CLIs and install status tracking hooks:

```bash
workmux setup
```

This detects Claude Code, Copilot CLI, OpenCode, and Pi by checking for their configuration directories, then offers to install the appropriate hooks. Workmux will also prompt you on first run if it detects an agent without status tracking configured.

Workmux automatically modifies your tmux `window-status-format` to display the status icons. This happens once per session and only affects the current tmux session (not your global config).

## Claude Code setup

If you prefer manual setup, install the workmux status plugin:

```bash
claude plugin marketplace add raine/workmux
claude plugin install workmux-status
```

Alternatively, you can manually add the hooks to `~/.claude/settings.json`. See [.claude-plugin/plugin.json](https://github.com/raine/workmux/blob/main/.claude-plugin/plugin.json) for the hook configuration.

## Pi setup

If you prefer manual setup, copy the workmux status extension to your global pi extensions directory:

```bash
mkdir -p ~/.pi/agent/extensions
curl -o ~/.pi/agent/extensions/workmux-status.ts \
  https://raw.githubusercontent.com/raine/workmux/main/.pi/extensions/workmux-status.ts
```

Restart pi for the extension to take effect.

## OpenCode setup

If you prefer manual setup, download the workmux status plugin to your global OpenCode plugin directory:

```bash
mkdir -p ~/.config/opencode/plugin
curl -o ~/.config/opencode/plugin/workmux-status.ts \
  https://raw.githubusercontent.com/raine/workmux/main/.opencode/plugin/workmux-status.ts
```

Restart OpenCode for the plugin to take effect.

## Codex setup

If you prefer manual setup, first ensure hooks are enabled in your Codex config:

```toml
# ~/.codex/config.toml
[features]
codex_hooks = true
```

Then download the hooks configuration:

```bash
curl -o ~/.codex/hooks.json \
  https://raw.githubusercontent.com/raine/workmux/main/.codex/hooks/workmux-status.json
```

If you already have a `~/.codex/hooks.json`, merge the hook entries from the downloaded file into your existing configuration.

Note: Codex hooks do not support detecting permission prompts, so only working/done states are tracked (no waiting state).

## Copilot CLI setup

If you prefer manual setup, copy the hooks configuration to your repository:

```bash
mkdir -p .github/hooks/workmux-status
curl -o .github/hooks/workmux-status/hooks.json \
  https://raw.githubusercontent.com/raine/workmux/main/.github/hooks/workmux-status/hooks.json
```

Note: Copilot CLI hooks are per-repository, unlike Claude Code and OpenCode which install globally. The Copilot CLI hooks API does not support detecting permission prompts, so only working/done states are tracked (no waiting state).

## Customization

You can customize the icons in your config:

```yaml
# ~/.config/workmux/config.yaml
status_icons:
  working: "🔄"
  waiting: "⏸️"
  done: "✔️"
  interrupted: "🛑"
```

## Interrupted agent detection

When an agent is in "working" status but its pane output hasn't changed for 10 seconds, workmux automatically detects it as interrupted. This typically happens when a user presses Ctrl+C to stop an agent. The interrupted status is shown in both the sidebar and dashboard with the interrupted icon (default: 🛑).

The detection runs in the sidebar daemon. If the agent resumes producing output, the interrupted indicator clears automatically. The dashboard reads the detection results from a shared runtime file, so both views stay in sync.

Tmux style codes are supported for colored icons, and work in both the tmux status bar and the dashboard:

```yaml
status_icons:
  done: "#[fg=#a6e3a1]󰄴#[fg=default]"
```

If you prefer to manage the tmux format yourself, disable auto-modification and add the status variable to your `~/.tmux.conf`:

```yaml
# ~/.config/workmux/config.yaml
status_format: false
```

```bash
# ~/.tmux.conf
set -g window-status-format '#I:#W#{?@workmux_status, #{@workmux_status},}#{?window_flags,#{window_flags}, }'
set -g window-status-current-format '#I:#W#{?@workmux_status, #{@workmux_status},}#{?window_flags,#{window_flags}, }'
```

## Jump to completed or waiting agents

Use `workmux last-done` to quickly switch to the agent that most recently finished its task or is waiting for user input. Repeated invocations cycle through all completed and waiting agents in reverse chronological order (most recent first).

Add a tmux keybinding for quick access:

```bash
# ~/.tmux.conf
bind l run-shell "workmux last-done"
```

Then press `prefix + l` to jump to the last completed or waiting agent, press again to cycle to the next oldest, and so on. This is useful when you have multiple agents running and want to quickly attend to agents that need your attention.

## Toggle between agents

Use `workmux last-agent` to toggle between your current agent and the last one you visited. This works like vim's `Ctrl+^` or tmux's `last-window` - it remembers which agent you came from and switches back to it. Pressing it again returns you to where you were.

This is available both as a CLI command and as the `Tab` key in the [dashboard](/guide/dashboard/).

Add a tmux keybinding for quick access:

```bash
# ~/.tmux.conf
bind Tab run-shell "workmux last-agent"
```

Then press `prefix + Tab` to toggle between your two most recent agents.
