---
description: Work with agents an external manager owns — view a session, take it over natively, hand it back
---

# agents

Lists agents across every runtime, and operates on the ones an external agent
manager (an ADE) owns rather than muxix.

```bash
muxix agents                 # list agents from every runtime
muxix agents view <ref>      # open or focus a manager's session
muxix agents takeover <ref>  # resume it in the provider's own CLI
muxix agents import <ref>    # hand a local session to a manager
muxix agents stop <ref>      # end an agent, whichever runtime owns it
```

A `<ref>` is the wire reference printed by `muxix agents`: `local:%12` for an
agent muxix runs itself, `paseo:cc842c69-…` for one a manager owns.

## view

Opens muxix's read-only view of a manager's session, or focuses it when it is
already open.

```bash
muxix agents view paseo:ca945abf-a86f-42ad-8645-d26726125ec2
# %119
```

Prints the pane id. This is the same action the sidebar performs when a session
is selected, exposed so switching does not require opening a TUI first — bind it
to a key:

```bash
bind p run-shell "muxix agents view paseo:ca945abf-a86f-42ad-8645-d26726125ec2"
```

Selecting the same session twice focuses the existing pane instead of opening a
second viewer. Records carry the pane's pid and are keyed per multiplexer
instance, so a pane id that has since been handed to something else is reopened
rather than focused — focusing a stranger's shell is worse than a fresh view.

Where the pane lands is `view.placement`; see
[agent runtimes](/guide/agent-runtimes) for that and for the view's keys.

**The view claims nothing.** Every call it makes is a stateless client call to
the manager's daemon, which owns the provider process and is the only writer. A
muxix view is therefore a peer of the manager's phone and desktop clients, and
all of them can use one session at the same time.

## takeover

A manager's session is also a provider session on disk, so it can be resumed in
the provider's own CLI — `claude --resume <handle>`, `pi --session <path>` —
giving a native TUI instead of a read-only view.

```bash
muxix agents takeover paseo:8b3c2bf4-7c21-4a09-80e2-c95bbce7894b --stop-ade
```

This is the one path that adds a **second** writer to one append-only
transcript, so it refuses unless the writers are serialized first:

```
paseo still holds a writer on this session, so resuming it locally would
leave two processes appending to one transcript.

Serialize first, by preference:
  1. ask paseo to release the session -- it exposes no such command today
  2. re-run with --stop-ade: stops its agent, interrupting work in flight,
     then `muxix agents import` hands the session back afterwards
  3. do nothing and keep using the session view, which claims nothing and
     works alongside every other device
```

`--stop-ade` is never implied: interrupting work someone started on another
device is your call. It stops the manager's agent for **that id alone** and
never every session on the host.

Muxix refuses rather than guessing a resume flag it has not verified, so a
provider with no known flag reports that and leaves the view working.

### Taking over from the view

Pressing `E` in a session view takes it over **in that pane**: the view
serializes the manager's writer and then replaces itself with the provider. The
session is therefore bound to the pane — quitting the provider, or its crashing,
ends the takeover and detaches, leaving nothing behind. A second takeover of one
session is impossible, because the process holding the key no longer exists.

Use this command instead when you want the session in a pane of its own;
`view.takeover_placement` governs where that lands.

### Takeover is a loan

The manager's agent survives a takeover untouched, so the session stays
resumable there — including from a phone. Muxix will not archive or delete it to
make the serialization bite harder: paseo's `archive` has no inverse in its CLI
and `delete` is a hard delete.

One caveat measured on paseo 0.11.2: `agent stop` is documented as a no-op for
an *idle* agent, and its daemon keeps the provider resident between turns. So on
an idle agent `--stop-ade` interrupts nothing, and driving the session from
another device afterwards can still append to the same transcript. Closing that
needs a release primitive paseo does not expose.

## import

Hands a local muxix session to a manager, so its desktop, web and phone clients
can drive it. The session does not move — the manager adopts the provider
session muxix already has, which is the same identifier takeover resumes.

```bash
muxix agents import %12
muxix agents import %12 --ade paseo   # when several are configured
# imported %12 into paseo as paseo:fad510dc-5053-4621-8314-0b6b30d0954f
```

`<ref>` is the local agent's pane id as the sidebar lists it (`%12`, or
`local:%12`). A manager's own ref is refused: import adopts a local session.

`--ade` is required only when more than one manager is configured, so a session
is never handed to the wrong product.

Before the import, muxix registers the session's directory as a workspace in
the manager (`workspaces` block), since `paseo agent import` has no workspace
flag and resolves it from the path.

Refused, with nothing sent to the manager:

- **No session id.** Import needs the provider session id, which the agent
  reports itself through `muxix signal --session-id` (the bundled pi and
  opencode integrations do; Claude Code's hooks do not yet). It is never
  guessed: a manager does not validate the handle it is given, so a wrong one
  would create a junk agent.
- **Unknown provider.** The agent's kind must be one the manager runs, listed in
  its `import_providers` (paseo: `claude`, `codex`, `copilot`, `opencode`, `pi`,
  `omp`). The error lists the accepted ones.

Importing the same session twice is refused by paseo itself (`Provider session
is already imported`).

::: warning paseo id mismatch
paseo 0.11.2 lists an imported agent under a different `id` in
`agent ls --json` than the one its commands accept; `muxix agents import` prints
the working one. Until paseo fixes it, an imported agent's sidebar row can fail
to open or delete. Use the printed ref with `muxix agents view`.
:::

## stop

Ends an agent, whichever runtime owns it.

```bash
muxix agents stop paseo:cc842c69-3470-4ad5-bb93-a13e4dfbca86
```

Stopping an agent the runtime has already dropped is not an error.

## Deleting

There is no `agents delete` command: deleting is `d` in the sidebar, behind a
confirmation, because a manager-side delete cannot be undone. See
[deleting agents](/guide/sidebar/#deleting-agents).

`stop` ends the process and leaves the session; delete removes the session.

## See also

- [Agent runtimes](/guide/agent-runtimes) — configuring a manager, the view's
  keys, pane placement, and reconnect behaviour.
