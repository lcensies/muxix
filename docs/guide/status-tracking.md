# Status tracking

Workmux can display the status of the agent in your tmux window list, giving you at-a-glance visibility into what the agent in each window is doing.

<div style="display: flex; justify-content: center; margin: 1.5rem 0;">
  <img src="/status.webp" alt="tmux status showing agent icons" style="border-radius: 4px;">
</div>

## Status icons

- 🤖 = agent is working
- 💬 = agent is waiting for user input
- ✅ = agent finished (auto-clears on window focus)

::: info
Currently only Claude Code supports hooks that enable this functionality. Gemini's support is [on the way](https://github.com/google-gemini/gemini-cli/issues/9070). Codex support can be tracked in [this issue](https://github.com/openai/codex/issues/2109).
:::

## Claude Code setup

Install the workmux status plugin:

```bash
claude plugin marketplace add raine/workmux
claude plugin install workmux-status
```

Alternatively, you can manually add the hooks to `~/.claude/settings.json`. See [.claude-plugin/plugin.json](https://github.com/raine/workmux/blob/main/.claude-plugin/plugin.json) for the hook configuration.

Workmux automatically modifies your tmux `window-status-format` to display the status icons. This happens once per session and only affects the current tmux session (not your global config).

## Customization

You can customize the icons in your config:

```yaml
# ~/.config/workmux/config.yaml
status_icons:
  working: "🔄"
  waiting: "⏸️"
  done: "✔️"
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
