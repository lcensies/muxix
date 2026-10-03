---
description: Declare providers, models, and context/compaction limits once and resolve them everywhere
---

# Models

The top-level `providers` section of `.workmux.yaml` is a **unified,
provider-centric model registry**. You declare each *provider* (the source that
serves a model) and the models it offers, along with context-window and
compaction limits. Declaring a model once lets the rest of workmux resolve its
limits by provider id or logical name instead of hard-coding them per agent — a
policy-style single source of truth.

```yaml
providers:
  anthropic:
    limit: 200000 # provider default context window (tokens)
    models:
      - name: sonnet
        id: claude-sonnet-4-6
      - name: sonnet-1m
        id: "claude-sonnet-4-6[1m]"
        limit: 1000000 # per-model override
      - name: opus
        id: claude-opus-4-8
  bedrock:
    limit: 200000
    models:
      - name: sonnet # same logical model, different source
        id: us.anthropic.claude-sonnet-4-6-v1:0
```

## Why provider-centric

The same logical model is frequently available from more than one source — for
example Anthropic direct **and** AWS Bedrock, each with a different
provider-specific id. Keying the registry by provider models this naturally: the
logical name `sonnet` appears under both `anthropic` and `bedrock`, and each
binding carries the id and limits that apply to that source.

## Schema

### Provider

| Field              | Type              | Description                                                       |
| ------------------ | ----------------- | ----------------------------------------------------------------- |
| `limit`            | integer, optional | Default context window (tokens) for this provider's models        |
| `compaction_limit` | integer, optional | Default compaction threshold (tokens) for this provider's models  |
| `base_url`         | string, optional  | Endpoint URL; may embed `{env:VAR}` references                    |
| `api_key_env`      | string, optional  | **Name** of the env var holding the API key (never the value)     |
| `npm`              | string, optional  | Adapter package for agents that load providers via npm (OpenCode) |
| `api`              | string, optional  | Wire protocol: `openai` (default) or `anthropic`                  |
| `options`          | map, optional     | Extra options passed through verbatim to agent provider config    |
| `models`           | list              | The models this provider serves                                   |

### Model

| Field              | Type              | Description                                                      |
| ------------------ | ----------------- | --------------------------------------------------------------- |
| `name`             | string            | Logical name/alias (e.g. `sonnet`); may repeat across providers |
| `id`               | string            | Provider-specific model id passed to that provider              |
| `tier`             | string, optional  | Routing hint, e.g. `low` / `medium` / `high`                    |
| `limit`            | integer, optional | Context window (tokens); overrides the provider default         |
| `compaction_limit` | integer, optional | Token count at which to compact; overrides the provider default |

## Limit resolution

Both limits resolve from the most specific value to the least:

- **Context limit**: model `limit` → provider `limit`.
- **Compaction limit**: model `compaction_limit` → provider `compaction_limit` →
  `context_limit × 0.85` (the default ratio) → unset.

So a model with only a `limit` of `200000` and no explicit compaction limit
compacts at `170000` tokens.

## Lookup

A model is resolved by **provider id** or **logical name**, case-insensitively:

- Resolving by a provider id (e.g. `us.anthropic.claude-sonnet-4-6-v1:0`) returns
  that exact binding.
- Resolving by a logical name (e.g. `sonnet`) may match multiple providers; the
  id-specific match is preferred when the key is an id.

This registry pairs with model-aware auto-compaction: the effective context
window is taken from the model actually in use (including 1M-window variants)
rather than a fixed default, so compaction is not triggered prematurely.

## Provider sync

A provider that declares any of `base_url`, `api_key_env`, or `npm` is a
**connectable** provider: `workmux setup` renders it into each supported
agent's native provider config, so one registry entry replaces N hand-edited
agent files. A provider without connection fields keeps its current
resolution-only role and is never written anywhere.

```yaml
providers:
  litellm:
    base_url: "{env:LITELLM_BASE_URL}"
```

Declare `api_key_env` — the *name* of the environment variable holding the
key — only when an agent needs a key reference written into its config; many
adapters (e.g. opencode-plugin-litellm) read the key from the environment on
their own, and omitting the field keeps key names out of the config entirely.

Renders as:

- **OpenCode** — a `provider.litellm` block in `~/.config/opencode/opencode.json`
  (`npm` defaults to `@ai-sdk/openai-compatible` for `api: openai`,
  `@ai-sdk/anthropic` otherwise; `api_key_env` renders as an `{env:...}`
  reference in `options.apiKey`;
  registry models project into `models.<id>` with `limit.context`). Only the
  `provider.<id>` keys workmux declares are touched — hand-written provider
  entries under other ids survive every sync. Caveat: OpenCode merges
  `opencode.jsonc` last, so a provider block there still wins.
- **Codex** — a `[model_providers.<id>]` table inside a
  `# workmux:providers begin`/`end` marker region of `~/.codex/config.toml`
  (`env_key` carries the variable name; `wire_api = "chat"`). Text outside the
  markers is never modified; edits inside them are overwritten on the next
  sync. Codex config has no env substitution, so a `base_url` embedding
  `{env:...}` — or `api: anthropic` — skips the provider with a warning.
- Other agents have no confirmed declarative provider format and are skipped
  (pi's `pi-provider-litellm` plugin reads the same env vars directly).

Secrets flow by environment variable **name** only: workmux never reads,
validates, or writes the values. The registry merges as a whole-field override
(project beats global), and sync writes global agent configs — so the same
provider id declared in two projects should mean the same backend
(last-synced project wins).

## See also

- [Agent bootstrap](./bootstrap.md) — plugins, skills, prompt components, features
- [Configuration](./configuration.md) — global vs. project config and merging
