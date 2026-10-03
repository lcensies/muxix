---
description: Sync local config with an org provision server, apply org policy, and route agents through a governed gateway
---

# provision

`workmux provision` syncs the machine with an **org provision server**: it pushes
a sanitised profile snapshot, fetches the org policy, caches it locally, audits
the current config against it, and (when the server advertises a gateway) points
the local coding agents at that gateway.

```bash
workmux provision              # same as `provision status`
workmux provision status       # show the cached policy, offline
workmux provision sync         # push profile -> fetch policy -> cache -> audit
workmux provision --dry-run    # sync without writing anything
```

Everything is opt-in: with no server configured, `provision status` reports
`not configured` and every other workmux command behaves as if the feature did
not exist.

## Options

| Flag        | Description                                                                     |
| ----------- | ------------------------------------------------------------------------------- |
| `--dry-run` | Fetch and audit, but write no policy cache, no agent config, and push no profile |
| `--strict`  | Exit non-zero on any policy violation or fetch failure (default: warn only)      |

Both flags apply to `sync`. `workmux provision --dry-run` with no subcommand
runs a dry-run sync; with no subcommand and no `--dry-run`, it runs `status`.

## Subcommands

| Subcommand | Description                                                            |
| ---------- | ---------------------------------------------------------------------- |
| `status`   | Print the cached policy (version, issue/expiry, allowlists). No network |
| `sync`     | Full round-trip against the server                                      |

## Configuration

The server URL and token resolve **from the environment first**, then from the
*global* config (`~/.config/workmux/config.yaml`). The env-var path exists so a
dev container or CI runner can provision with no config file at all.

```yaml
# ~/.config/workmux/config.yaml
provision:
  server_url: https://provision.corp.example.com
  token: ${env:MY_ORG_TOKEN} # or use token_path below
  token_path: ~/.config/workmux/provision-token # must be mode 600
  grace_period_secs: 259200 # keep using an expired policy for 72h
```

| Setting                    | Env var (wins)      | Default  | Description                                             |
| -------------------------- | ------------------- | -------- | ------------------------------------------------------- |
| `provision.server_url`     | `WORKMUX_PROVISION_URL`   | –  | Provision server root                                   |
| `provision.token`          | `WORKMUX_PROVISION_TOKEN` | –  | Bearer token, or a `${env:VAR}` / `${file:/path}` reference to one |
| `provision.token_path`     | `WORKMUX_PROVISION_TOKEN` | –  | File holding the bearer token; must be mode `600`       |
| `provision.grace_period_secs` | –                | `259200` | How long an expired policy stays usable before it is dropped |

::: warning Token file permissions
A `token_path` that is group- or world-readable is rejected with an error
rather than read. `chmod 600` it.
:::

::: info Accepted but not yet honored
`provision.sync_on_setup`, `provision.policy_ttl_secs`, and
`provision.insecure_skip_tls` parse without error but are not wired up today.
Policy TTL comes from the server's `ttl_seconds`; `workmux setup` never syncs
on its own — run `workmux provision sync` explicitly.
:::

## What `sync` does

1. **Push profile** — `POST /api/provision/profile` with the snapshot described
   under [Profile snapshot](#profile-snapshot). A push failure is non-fatal;
   the fetch still runs.
2. **Fetch policy** — `GET /api/provision/policy`. Under `--strict`, a fetch
   failure aborts; otherwise it warns and returns.
3. **Audit** — the policy is applied to the merged config and every violation is
   reported. Under `--strict`, violations fail the command.
4. **Cache** — the policy is written atomically to
   `~/.config/workmux/policy.yaml`, stamped with `issued_at` / `expires_at`.
5. **Configure agents** — when the response carries a `gateway`, the local
   agents are pointed at it (see [Gateway wiring](#gateway-wiring)).
6. **Audit log** — one JSON line is appended to
   `~/.local/state/workmux/provision-audit.jsonl`.

`--dry-run` stops after step 3.

## How policy is enforced

The cached policy is applied on **every config load**, not only during `sync`.
`Config::load` reads `policy.yaml`, merges the locked fields over the user's
config, and prints violations. So a policy takes effect for `workmux add`,
`dashboard`, `pipeline`, and everything else without a re-sync.

| Policy field              | Effect                                                                   |
| ------------------------- | ------------------------------------------------------------------------ |
| `locked.proxy_chain`      | Replaces `proxy_chain` entirely; a local value is a violation             |
| `locked.sandbox_network`  | Replaces `sandbox.network`; a local `policy`/`allowed_domains` violates   |
| `forbidden_mcp_commands`  | Substring patterns that must not appear in any MCP server `command`       |
| `allowed_mcp_servers`     | When non-empty, MCP servers outside the list are violations               |
| `allowed_agent_kinds`     | When non-empty, restricts `agent:` to those CLI stems (`claude`, `codex`) |
| `deny_external_providers` | With `allowed_providers`, flags any `providers:` entry off the allowlist  |
| `violation_severity`      | `warn` (default) or `error` — the severity stamped on each violation      |
| `team_profile_url`        | Where `workmux profile diff` would fetch the team baseline                |

Locked fields are **applied regardless of severity** — the severity only decides
how loudly the override is reported. Violations are always printed to stderr
when attached to a terminal, and always logged.

### Cache freshness

| State     | Meaning                                     | Behavior                                        |
| --------- | ------------------------------------------- | ----------------------------------------------- |
| `Fresh`   | `now <= expires_at`                         | Applied silently                                |
| `Stale`   | Within `grace_period_secs` past expiry      | Still applied, with a warning                   |
| `Expired` | Past the grace period                       | Not applied; warns to run `provision sync`      |
| `Missing` | No `policy.yaml`                            | No-op                                           |

`provision status` uses a fixed 72-hour grace window; config load uses the
configured `grace_period_secs`.

## Gateway wiring

When the fetch response includes a `gateway`, `sync` writes native provider
config so agent traffic routes through it without hand-wiring
`ANTHROPIC_BASE_URL` / `OPENAI_BASE_URL`:

| Agent       | File                                | What is written                                                                 |
| ----------- | ----------------------------------- | ------------------------------------------------------------------------------- |
| OpenCode    | `~/.config/opencode/opencode.json`  | A provider block (`@ai-sdk/openai-compatible`) named by the server, defaulting to `provision-gateway`, with `baseURL: <gw>/v1`, plus `model` when unset |
| Claude Code | `~/.claude/settings.json`           | `env.ANTHROPIC_BASE_URL` and an `apiKeyHelper` that echoes the token env var     |

**Secrets are never written to disk.** The token is referenced by env var name —
OpenCode's `{env:VAR}` interpolation and Claude's `apiKeyHelper` both resolve it
at runtime. The var defaults to `WORKMUX_PROVISION_TOKEN` when the gateway does not
name one. Both files are merged: only the keys above are touched.

## Profile snapshot

The snapshot is deliberately thin and carries **no secrets** — no API keys, no
env values, no MCP commands:

| Field                          | Content                                                    |
| ------------------------------ | ---------------------------------------------------------- |
| `workmux_version`, `platform`  | Build and OS                                                |
| `agent_kind`                   | CLI stem of `agent:` (`claude`), path and args stripped     |
| `mcp_names`                    | MCP server **names** only                                   |
| `provider_names`               | Provider prefixes taken from agent definitions' `model:`    |
| `features`                     | `sandbox_enabled`, `proxy_chain_enabled`, `bootstrap_enabled` |
| `hostname_hash`                | djb2 hash of the hostname — dedup without machine identity  |

### `workmux profile`

Inspect and share that snapshot without talking to a server:

```bash
workmux profile show              # print the snapshot as YAML
workmux profile export -o me.yaml # write it to a file (stdout when -o is omitted)
workmux profile diff              # compare against the org's team profile
```

`profile diff` currently reports the `team_profile_url` from the cached policy;
downloading and diffing the team baseline is not implemented yet.

## Files

| Path                                            | Content                       |
| ----------------------------------------------- | ----------------------------- |
| `~/.config/workmux/policy.yaml`                 | Cached org policy             |
| `~/.local/state/workmux/provision-audit.jsonl`  | One JSON line per sync        |

Both honor `XDG_CONFIG_HOME` / `XDG_STATE_HOME`.

## Backends

The policy does not have to come from an HTTP server. `provision.backend`
selects where it comes from; everything downstream — caching, layering,
auditing — is identical.

| Backend | Config | For |
| --- | --- | --- |
| `http` (default) | `server_url`, `token` / `token_path` | A provision server |
| `file` | `path` | Air-gapped machines; a policy dropped on disk by config management |
| `exec` | `command`, `timeout_secs` | Orgs whose own tool already knows how to authenticate |

```yaml
provision:
  backend: file
  path: /etc/workmux/policy.json
```

```yaml
provision:
  backend: exec
  command: /usr/local/bin/fetch-workmux-policy
  timeout_secs: 30
```

The `exec` backend reads the policy from the command's stdout, surfaces its
stderr on a non-zero exit, and kills it on timeout. It runs only when a config
explicitly names it — no more privileged than the config file already is, but
worth knowing.

A policy document may be the full fetch envelope (`{policy, version, ...}`) or a
bare policy object, in JSON or YAML. A hand-written file should not have to
mimic an HTTP envelope it never travelled in.

### Endpoints

For `backend: http`, the request paths are configurable so your server need not
adopt any particular route layout:

```yaml
provision:
  endpoints:
    policy: /v2/harness-policy
    profile: null     # explicit null: do not push a machine profile at all
```

Unset paths default to `/api/provision/policy` and `/api/provision/profile`.

### Schema version

A policy carries a `schema_version`. One newer than the running workmux
understands is rejected, naming both versions, and nothing is cached — better
than half-applying semantics from a newer contract. Unknown fields within a
supported version are ignored, so a server can add fields without breaking older
clients.

## How a policy reaches the config

A policy is not applied as a patch after the merge. Its parts become config
layers, at deliberately opposite ends of the stack:

- **`defaults`** rank *below* [profiles](../../guide/profiles.md) and CLI flags.
  A default is a suggestion for a machine that has not decided; it must not
  silently beat a deliberate project choice.
- **`locked`** ranks *above* everything, including CLI flags. A lock is the one
  thing an organization can actually rely on.

Which means you can ask where a value came from:

```bash
workmux config resolve --explain
```

```yaml
merge_strategy: rebase  # from org policy v-layered (defaults)
    policy: deny  # from org policy v-layered (locked)
```

`workmux provision sync` names every setting a lock overrides, so an overridden
value is reported at the moment you sync rather than discovered later.

### Offline behaviour

The last policy obtained is cached. While fresh it applies silently; past its
TTL it still applies with a warning; past `grace_period_secs` it contributes
nothing and workmux reports that a re-sync is required. **No command other than
`workmux provision` makes a network request for policy** — the cache is read
locally on every config load.

## Migrating from `WORKMUX_SC_*`

The `WORKMUX_SC_URL` and `WORKMUX_SC_TOKEN` variables were renamed to
`WORKMUX_PROVISION_URL` and `WORKMUX_PROVISION_TOKEN` so the names describe the
capability rather than one vendor's product. The old names still work for one
release and print a deprecation warning; when both are set, the new name wins.

```bash
# before
export WORKMUX_SC_URL=...
export WORKMUX_SC_TOKEN=...

# after
export WORKMUX_PROVISION_URL=...
export WORKMUX_PROVISION_TOKEN=...
```

## Examples

```bash
# One-time setup on a corporate machine
export WORKMUX_PROVISION_URL=https://provision.corp.example.com
export WORKMUX_PROVISION_TOKEN=$(cat ~/.secrets/provision-token)
workmux provision sync

# CI gate: fail the build on any policy violation
workmux provision sync --strict

# See what a sync would change, touching nothing
workmux provision --dry-run

# Offline check of the cached policy
workmux provision status
```

## See also

- [setup](./setup.md) — install agent hooks, skills, and the bootstrap manifest
- [Agent bootstrap](../../guide/bootstrap.md) — the per-project agent manifest
- [Models](../../guide/models.md) — the provider/model registry policy audits
