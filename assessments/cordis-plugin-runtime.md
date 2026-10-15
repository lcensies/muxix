# Cordis as muxix's plugin runtime — technical assessment

Status: assessment, not a proposal. Recommendation at the end is **not now**,
with the conditions that would change it stated explicitly.

Scope: whether to adopt [cordiverse/cordis](https://github.com/cordiverse/cordis)
as muxix's plugin runtime, in pursuit of dsh's "everything is a plugin"
architecture. Includes a second, independent section revising the ADE session
design around concurrent multi-device use, because that requirement was
established while researching this one.

Everything cited from muxix is from this tree at `0.1.211`. Everything cited
from paseo is measured against `0.11.2` on this machine.

---

## 1. What cordis actually provides

Cordis describes itself as "A Meta-Framework of Spatiotemporal Composability",
with an accompanying paper, *A Programming Paradigm for Spatiotemporal
Composability*. Its two named axes:

- **Temporal composability** — completely revert a component's side effects when
  it is removed.
- **Spatial composability** — declare and reactively manage inter-component
  dependencies.

The concrete model, five parts:

1. **Context** (`ctx`) — a dependency container, implemented as a proxy whose
   property reads go through a service resolver.
2. **Service** — a named capability mounted on a stable context key (`ctx.tools`,
   `ctx.llm`, `ctx.sessions`).
3. **Plugin** — a function or object with optional `inject` and `apply(ctx)`, or
   a `Service` subclass the runtime mounts into the current context.
4. **`inject`** — a plugin names services it needs; its body is deferred until
   they exist, and is unloaded and re-run when one changes.
   `ctx.inject(deps, cb)` returns a fiber that settles once loading finished.
5. **Typed events** — coordination between plugins without direct references.

In dsh this buys something real: the model adapter, tool registry, session log
and the agent loop itself are each a plugin, so each is replaceable from
configuration, and a model-facing `cordis_inspect` tool reads the live graph.
dsh depends on a vendored fork (`@deepseek-ai/cordis`), not upstream.

Upstream status matters for this assessment. The README states: "Cordis is under
active development. The API is not yet stable and may change without notice."
Current line is `4.0.0-rc.11`, and release notes show breaking renames still
landing within the rc series — config resolution split out of `internal/update`,
update listeners losing an argument, persistence handlers moved to
`internal/commit`.

---

## 2. muxix's real extension points

muxix already has extension seams. They are **trait objects behind explicit
registries, plus declarative config templates** — not a service container.

### 2.1 Agent runtimes — the closest thing to a cordis service

`src/agent/runtime/mod.rs:329`:

```rust
pub trait AgentRuntime: Send + Sync {
    fn name(&self) -> &str;
    fn features(&self) -> RuntimeFeatures;
    fn health(&self) -> RuntimeHealth;
    fn start(&self, req: &StartRequest) -> Result<AgentRef>;
    fn send(&self, agent: &AgentRef, text: &str) -> Result<()> { /* default: unsupported */ }
    fn status(&self, agent: &AgentRef) -> Result<RuntimeStatus>;
    fn list(&self) -> Result<Vec<RuntimeAgent>>;
    fn stop(&self, agent: &AgentRef) -> Result<()>;
    fn import(&self, session: &ExternalSession) -> Result<AgentRef> { /* default: unsupported */ }
}
```

`src/agent/runtime/registry.rs:28`:

```rust
pub struct RuntimeRegistry {
    runtimes: BTreeMap<String, Arc<dyn AgentRuntime>>,
}
```

This is already a registration seam with a runtime-mutable map
(`register(&mut self, runtime: Arc<dyn AgentRuntime>)`, `:50`) and name-keyed
resolution (`get`, `:58`). `for_config` (`:42`) populates it from configuration:

```rust
pub fn for_config(cfg: &crate::config::Config) -> Self {
    let mut reg = Self::new();
    for (name, ade) in crate::projects::sync::effective_ades(cfg) {
        reg.register(Arc::new(super::super::ade::AdeRuntime::new(name, ade)));
    }
    reg
}
```

Mapped onto cordis: `AgentRuntime` is a **service definition**, each impl is a
**service provider**, the registry is a degenerate **context** with one service
category, and `for_config` is a **config-driven loader**. The structure cordis
would supply for this axis is already here; what is missing is only that
providers must be Rust types compiled in.

`RuntimeFeatures` (returned by `features()`) is worth noting as a capability
descriptor — muxix already models "what can this provider do" as data rather
than as trait-presence, which is the same thing cordis's capability inspection
does.

### 2.2 Declarative providers — the axis that needs no code at all

`AdeConfig` (`src/config.rs`) makes a whole agent-manager integration a config
record. The paseo preset (`AdeConfig::paseo()`, `src/config.rs:2971`) declares
`command`, `start_args`, `send_args`, `list_args`, `stop_args`, `import_args`,
`id_field`, `status_field`, `status_map`, `owns_worktree`, `projects`, and
`workspaces` — all arg templates and field pointers rendered by one generic
`AdeRuntime` (`src/agent/ade.rs:27`, `impl AgentRuntime` at `:245`).

This is the single most important fact in the assessment: **muxix's existing
extensibility story is data-driven, and it already delivered a complete
third-party integration with zero plugin framework.** A second ADE needs a
config block, not code and not a plugin.

`AdeConfig::presets()` keys those blocks by name, and `merge_over` lets a user
block overlay a preset field by field — a provider-selection mechanism
equivalent to choosing a cordis service provider from configuration.

### 2.3 Agent profiles — behaviour variation by trait default

`src/agent/profile.rs:10`:

```rust
pub trait AgentProfile: Send + Sync {
    fn name(&self) -> &'static str;
    fn needs_bang_delay(&self) -> bool { false }
    fn needs_auto_status(&self) -> bool { false }
    fn skip_permissions_flag(&self) -> Option<&'static str> { None }
    fn prompt_flag(&self) -> Option<&'static str> { Some("--") }
    fn default_subcommand(&self) -> Option<&'static str> { None }
    fn auto_name_command(&self) -> Option<&'static str> { None }
    fn continue_flag(&self) -> Option<&'static str> { None }
    fn model_flag(&self) -> Option<&'static str> { None }
    fn permission_mode_flags(&self, _mode: &str) -> Option<Vec<String>> { None }
}
```

Ten knobs, all defaulted, with per-agent impls (`ClaudeProfile:115`,
`OpenCodeProfile:182`, `CodexProfile:206`, and others). Cordis has no analogue
to add here: this is a variation table, and the lazy direction for it is to
migrate the knobs into `AgentDefinition` YAML — moving *toward* data, away from
code. A plugin runtime moves the opposite way.

### 2.4 Multiplexer backends

`src/multiplexer/mod.rs:35` — `pub trait Multiplexer: Send + Sync`, selected by
`BackendType` (`src/multiplexer/types.rs:201`: Tmux, WezTerm, Kitty, Zellij)
through `detect_backend()` (`:1010`), `resolve_backend()` (`:1031`) and
`create_backend()` (`:1052`).

A closed enum with a detect-and-construct function. Making this plugin-pluggable
is possible but has no demand: the set of terminal multiplexers is small, known,
and changes about once a year.

### 2.5 Declarative item registries with remote distribution

`src/agent/registry.rs:33`:

```rust
pub enum RegistrySource {
    Local { local: Option<String> },
    Git { git: GitRegistrySource },
    Url { url: String },
}
```

`AgentRegistry::load` (`:84`) resolves inline `agent_defs`, then configured
registries, then a default directory, earlier entries winning.

muxix therefore **already has a third-party distribution model** — local dir,
git repo, or URL — for one class of extension (agent definitions). Any plugin
story should reuse this rather than inventing a second one, and notably it
achieves distribution without a plugin *runtime*.

### 2.6 Events — the actual gap

`src/signals/event.rs` defines the `muxix_evt!` macro and a filter consulted on
each emission (`:75`). Emissions land in the log file via `tracing`. Current
call sites, 18 of them, include `cli.rs:1213` / `1220` (`config.load`),
`cli.rs:1253` (`cmd.dispatch`), `main.rs:47` / `57` / `64`
(`app.*`), `command/signal.rs:78` / `113` / `150` / `177` (`signal.write`),
`command/set_window_status.rs:46` (`hook.status`), `tasks/graph.rs:815`,
`workflow/setup.rs:573`, `multiplexer/util.rs:177`.

**There are no subscribers.** `muxix_evt!` is an observability sink, not a bus.
Cordis's typed events have nothing to connect to here, and neither would a
hand-rolled plugin layer: no component outside the process can observe muxix
doing anything.

This is the one genuine blocker for any plugin story, and it is independent of
cordis.

### 2.7 Long-lived process surface

muxix is overwhelmingly one-shot: `main.rs` emits `app.start`, dispatches
(`cli.rs:1253`), emits `app.done` (`:57`) or `app.fail` (`:64`), exits.

The exception is the sidebar daemon (`src/command/sidebar/daemon.rs`). Its
`SocketServer` (`:147`) binds a `UnixListener` with `0o600` permissions
(`:154`), and `broadcast` (`:203`) writes a length-prefixed `serde_json`
`SidebarSnapshot` to connected clients, caching the last payload for new
arrivals. Lifecycle is in `daemon_ctrl.rs`: `ensure_daemon_running` (`:13`),
`spawn_daemon` (`:32`, re-execs `self` with `_sidebar-daemon`), `kill_daemon`
(`:67`, SIGTERM), `signal_daemon` (`:80`, SIGUSR1 for immediate refresh).

A second long-lived surface exists in `src/sandbox/rpc.rs:97` — `RpcServer`
binding `0.0.0.0:0`, token-authenticated, `MAX_CONNECTIONS = 16` (`:121`).

So muxix has two precedents for "long-lived process with a socket and a JSON
frame protocol", and zero precedents for "long-lived in-process component graph".

### 2.8 Wiring style today

`src/command/remove.rs:40` is representative:

```rust
let config = config::Config::load(None)?;
let mux = create_backend(detect_backend());
let context = WorkflowContext::new(config, mux, None)?;
```

Dependencies are constructed at the entry point and passed explicitly. There is
no resolver, no late binding, and no place where a component asks for a
capability by name and waits for it to appear.

### 2.9 Summary map

| Cordis concept | muxix today | Citation |
|---|---|---|
| Context (container + proxy resolver) | none; explicit construction + pass-down | `src/command/remove.rs:40` |
| Service definition | `AgentRuntime` trait | `src/agent/runtime/mod.rs:329` |
| Service provider | `LocalRuntime`, `AdeRuntime` | `src/agent/ade.rs:245` |
| Service registry keyed by name | `RuntimeRegistry` `BTreeMap<String, Arc<dyn …>>` | `src/agent/runtime/registry.rs:28` |
| Config-driven provider loading | `RuntimeRegistry::for_config` | `registry.rs:42` |
| Provider selection from config | `AdeConfig::presets` + `merge_over` | `src/config.rs:2971` |
| Capability descriptor | `RuntimeFeatures` | `src/agent/runtime/mod.rs` |
| Behaviour variation | `AgentProfile` trait defaults | `src/agent/profile.rs:10` |
| Swappable backend | `Multiplexer` + `BackendType` | `src/multiplexer/mod.rs:35` |
| Third-party distribution | `RegistrySource::{Local,Git,Url}` | `src/agent/registry.rs:33` |
| Typed event bus | `muxix_evt!` → log only, **no subscribers** | `src/signals/event.rs:75` |
| Reversible teardown (fibers) | process exit; daemon SIGTERM | `daemon_ctrl.rs:67` |
| `inject` dependency resolution | nothing to resolve — see §3.1 | — |

---

## 3. What adopting cordis would require

### 3.1 The topology problem, first

Cordis's `inject` earns its complexity when plugins depend on **each other's**
services and the runtime must order, defer, and re-run them as those services
appear and vanish.

muxix's components depend on muxix — config, a `Multiplexer` handle, the state
store, the task graph. They do not depend on one another. `AdeRuntime` does not
consume a service another plugin provides; it renders arg templates and shells
out. That is a **star**, and `inject` has nothing to resolve on a star.

dsh's graph is not a star — its tool registry, model adapter and agent loop are
mutually referential and individually swappable, which is exactly when the
machinery pays. Adopting the framework without the topology buys the cost
without the benefit.

### 3.2 Module loading

muxix is a single statically-linked Rust binary. Resources are compiled in with
`include_str!` from `resources/` (per `CLAUDE.md`). Distribution is
`cargo-binstall` with six per-target overrides (`Cargo.toml:120-140`) plus Nix.

Cordis loads ES modules at runtime through `import()`. Two ways to host that:

**(a) Embed a JS engine in-process** (`deno_core`, `rquickjs`).
`tokio = { version = "1", features = ["full"] }` is already a dependency
(`Cargo.toml:80`), so the async substrate exists — this is less absurd than it
would be in a purely synchronous CLI. Costs: V8 via `deno_core` adds substantial
compile time and tens of MB to a binary distributed through binstall; `rquickjs`
is far lighter but is not the runtime cordis is developed against (Node APIs,
`import()` semantics, HMR), so cordis-on-QuickJS is itself a porting project.

**(b) Node sidecar process.** Requires Node at runtime. Today there is **no root
`package.json`** — the only ones are `docs/package.json` and
`resources/opencode/package.json` — and **no `nodejs`, `bun`, or `deno` in
`flake.nix` or `shell.nix`**. So this adds a language runtime to muxix's runtime
contract, which propagates into the Nix package, the binstall story, `just
check` (which already runs format → clippy → unit tests → python lint → docs
build), and the sandbox (`src/sandbox/`).

Either way, muxix's own extension distribution (`RegistrySource::{Local,Git,Url}`,
`src/agent/registry.rs:33`) would need a second, npm-shaped sibling, because
cordis plugins are npm packages.

### 3.3 Lifecycle and teardown

This is where the mismatch is sharpest. Cordis's headline feature is complete
reversal of a component's side effects on removal — valuable when components
mount and unmount inside a living process.

muxix's dominant lifecycle is `main` → dispatch → exit (`main.rs:47-64`).
Teardown is `_exit`. There is no component graph whose unmount ordering could be
wrong, and nothing to revert that the OS does not revert.

The sidebar daemon is the only long-lived process, and its teardown is a signal
(`kill_daemon`, `daemon_ctrl.rs:67`) plus a stale-socket sweep on next start
(`ensure_daemon_running`, `:18-24`). Adopting fibers would mean first *creating*
a long-lived host whose lifetime justifies them. The framework's main benefit is
therefore contingent on building the thing that would need it.

### 3.4 Inter-component wiring

Every seam exposed to a plugin needs a cross-language contract. Today the seams
are Rust traits over Rust types: `&AgentRef`, `RuntimeAgent`, `StartRequest`,
`RuntimeFeatures`, `RuntimeHealth`, `RuntimeStatus`, `ExternalSession`
(`src/agent/runtime/mod.rs`). Exposing `AgentRuntime` to JS means:

- a serialization contract for each of those types (they are `serde`-friendly,
  so this is mechanical but not free),
- a **versioned** boundary, because a plugin compiled against one shape must not
  silently mis-drive a later muxix,
- an error-mapping convention — `anyhow::Result` with context chains does not
  survive a JSON hop intact,
- a decision on blocking: `AgentRuntime` methods are synchronous and called from
  TUI code paths (`foreign_agent_panes`, `registry.rs:198`, is called during
  snapshot building). A JS-backed provider makes those calls async or makes them
  block the render loop.

That cost is attributable to **the boundary, not to cordis**. It is identical for
a hand-rolled layer. Cordis neither adds nor removes it.

### 3.5 Scope boundary

`CLAUDE.md` is explicit that muxix owns deterministic plumbing and does not
schedule work: "There is no pipeline runner, no task loop, no decomposition
engine… New 'run this DAG for me' features belong in the harness, not here."

A plugin kernel with a service container, an event bus, and reactive reload is
harness-shaped. That is not fatal — plugins would extend plumbing, not schedule —
but it means the "everything is a plugin" framing imports an architecture whose
natural centre of gravity sits outside this repo's stated scope. dsh is an agent
harness; muxix explicitly is not.

---

## 4. Comparison

### 4.1 Cordis vs a hand-rolled plugin layer

A hand-rolled TS loader, in full:

```ts
const disposers = []
for (const spec of config.plugins) {
  disposers.push(await (await import(spec)).default(ctx) ?? (() => {}))
}
```

| Capability | Cordis | Hand-rolled |
|---|---|---|
| Load plugins from config | yes | yes, above |
| Teardown hook | fibers, ordered, reversible | disposer array, reverse order |
| Dependency ordering (`inject`) | yes, reactive | none — unnecessary on a star (§3.1) |
| Hot reload | yes | no; restart the host |
| Service keys / late binding | yes | plain object properties |
| Upstream stability | `4.0.0-rc.11`, "may change without notice" | you own it |
| Vendoring burden | dsh vendors it; you would too | none |
| Lines of infrastructure | a framework | ~10 |

Decisive asymmetry: keeping `ctx` cordis-shaped (`ctx.on(topic, fn)`,
`ctx.plugin(p)`, plugin returns a disposer) makes cordis a **later drop-in**. The
expensive, hard-to-reverse decisions are the host location and the Rust↔JS
boundary (§3.2, §3.4); the DI framework inside the host is a swap. Choosing
hand-rolled first does not foreclose cordis; choosing cordis first does commit
to vendoring an rc-stage framework.

### 4.2 Cordis vs staying as-is

What status quo already extends without touching core:

- a new agent manager — `AdeConfig` block (`src/config.rs:2971` as the template)
- a new agent definition — YAML, distributable by git or URL
  (`src/agent/registry.rs:33`)
- per-profile agent deltas — `AgentProfileAgent` (`src/config.rs:609`):
  `add_plugins`, `exclude_plugins`, `add_skills`, `add_prompt_components`,
  `exclude_paths`, RFC 7386 `settings` patch
- external reactions — hooks and signal files (`src/signals/`)
- a new agent runtime — `RuntimeRegistry::register` (`registry.rs:50`), currently
  compile-time only

What status quo cannot do:

- add a subcommand without editing `src/cli.rs`
- contribute a sidebar or dashboard widget — `SidebarSnapshot`
  (`src/command/sidebar/snapshot.rs`) is a fixed struct
- observe muxix at all — `muxix_evt!` has no subscribers (§2.6)
- hold state across invocations without reimplementing the daemon pattern

Note that only the last three are plugin-runtime problems, and the third is a
prerequisite for the other two.

---

## 5. Recommendation

**Do not adopt cordis now.** Reasons, in order of weight:

1. **The dependency graph is a star** (§3.1). `inject` — cordis's central
   mechanism — has nothing to resolve. The framework's value is proportional to
   inter-plugin dependency, which is currently zero.
2. **The lifecycle that justifies fibers does not exist** (§3.3). muxix is
   one-shot; the only daemon is a snapshot broadcaster. Reversible teardown would
   have to be preceded by building the long-lived host that needs it.
3. **It adds a language runtime to the runtime contract** (§3.2). No root
   `package.json`, no node/bun/deno in `flake.nix`, distribution through binstall
   with six target overrides.
4. **Upstream is explicitly unstable** — `4.0.0-rc.11`, "the API is not yet
   stable and may change without notice", with breaking renames inside the rc
   series. dsh insulates itself by vendoring; muxix would inherit that burden.
5. **Deferring is nearly free** (§4.1), while adopting is not reversible at the
   same price.

**Do this instead, in order.** Each step is independently useful and none
forecloses cordis:

1. **Make `muxix_evt!` subscribable.** Fan emissions out on a socket, reusing the
   sidebar daemon's existing framing (`SocketServer::broadcast`,
   `daemon.rs:203`). This is the real blocker (§2.6) and it is pure Rust. Until
   it exists, no plugin of any kind can observe muxix.
2. **Subprocess plugins.** `muxix-<name>` resolved on `PATH`, JSON over stdio,
   distributed through the mechanism that already exists
   (`RegistrySource::{Local,Git,Url}`). Zero added runtime, any language. This is
   also the architecture paseo itself uses to drive agents (§6.1) — process plus
   stdio JSON contract — so it is the validated shape, not a compromise.
3. **Push `AgentProfile` knobs into `AgentDefinition` YAML.** Removes the most
   common reason to touch Rust for a new agent, in the data direction muxix
   already prefers (§2.3).
4. **Only then, if a plugin must hold long-lived state across events:** a TS
   sidecar with the ~10-line loader and a cordis-shaped `ctx`.

**Revisit cordis when all three hold:**

- two or more plugins consume each other's services (the star becomes a graph),
- plugins are swapped at runtime by people who did not write them,
- cordis has a stable, non-rc release.

Until then the honest summary is: muxix's extensibility problem is an **event
bus and a subcommand seam**, not a dependency-injection container.

---

## 6. Multi-device session use — hard requirement and design revision

This section is independent of the cordis question. It records a requirement and
corrects the ADE session design in `openspec/changes/ade-session-bridge`.

**Requirement.** Phone and desktop must both be able to use one session.
Concurrent use is the target; temporally interleaved re-attach from either device
is the minimum acceptable. Exclusive ownership transfer is **one permitted
implementation of the optional native-takeover path, not the requirement**.

### 6.1 Measured facts

Process tree under the paseo daemon (`ps -eo pid,ppid,etime,cmd`):

```
   2558  Paseo Supervisor
  18916  Paseo Daemon                       (child of 2558)
 131281  claude --output-format stream-json --verbose --input-format stream-json
         --permission-prompt-tool stdio --include-partial-messages ...   (28:57, child of 18916)
 123352  pi --mode rpc --session <...>.jsonl
         --extension /tmp/paseo-pi-extension-*/paseo-integration.mjs    (38:52, child of 18916)
```

Four conclusions:

1. **The provider process is resident, and "idle" does not mean "no writer."**
   Agent `cc842c6` reports `Status: idle`, yet its `claude` process has been up
   28:57 holding the session.
2. **The daemon is the sole writer.** It owns that process's stdin and drives it
   with `--input-format stream-json`. Clients do not write to the session; they
   ask the daemon to.
3. **Permissions ride the same channel** (`--permission-prompt-tool stdio`),
   which is why `paseo permit ls|allow|deny` exists as a client-side surface.
4. **Concurrency is already solved for daemon clients.** Phone, desktop and CLI
   are peers of one worker (`paseo status`: `listen 127.0.0.1:6767`, plus a relay
   endpoint). One writer, N clients.

Corroborating: the agent record carries no ownership field whatsoever —
`paseo inspect --json` returns `Id, Name, Provider, Model, Thinking, Status,
Archived, ArchivedAt, Mode, Cwd, CreatedAt, UpdatedAt, LastUsage, Capabilities,
AvailableModes, PendingPermissions, Worktree, ParentAgentId`. No `owner`,
`lease`, `lock`, or `tty`. And `paseo agent reload` "restarts the underlying
process", confirming the process is disposable while the session file is the
durable artifact.

### 6.2 What this corrects

The requirement is **already satisfied today for phone and desktop**, and muxix
was never the obstacle. The obstacle would be muxix *becoming a second writer* by
running `claude --resume <handle>` against a session whose resident process is
still live — two OS processes appending to one `.jsonl`.

The previous design made native takeover the centrepiece and declared "the ADE
agent must be idle, stop it first" a hard rule. That was wrong twice over: idle
does not imply no writer (§6.1.1), and gating on exclusivity imposed a constraint
the requirement does not ask for.

### 6.3 Revised design

**Default — muxix joins as a peer client. Concurrent, no exclusivity.**

muxix drives the session through the daemon exactly as the phone and desktop do:
view by `logs -f`, input by `send` (already wired as `AdeConfig.send_args`),
permissions by `permit allow|deny`, mode by `agent mode`, interrupt by `stop`
(already wired as `stop_args`). No client claims anything, so phone, desktop and
muxix are usable at the same moment. This **exceeds** the stated minimum, and it
is the path that must never require a baton.

**Optional — native takeover, for the local-TUI experience.**

Resuming the session in the provider's own CLI (`claude --resume <handle>`, where
the handle is `persistence.nativeHandle`) does introduce a second writer, so this
path alone must serialize writers for that one session, and must be reversible so
either device can re-attach afterwards. Three implementations, in preference
order:

- **A — baton (preferred, needs upstream).** Ask the daemon to release or park
  its process for that session; muxix resumes natively; on exit the daemon
  re-acquires. Consistent with paseo's own model, given `agent reload` already
  tears down and recreates the process around a surviving session. Requires a
  release primitive paseo does not currently expose.
- **B — interrupt and re-register (available today).** `paseo stop`, resume
  natively, then `paseo import <handle>` to hand it back. Coarser: `stop`
  interrupts in-flight work rather than parking it. **This is the demoted
  "stop the other agent first" — one option, not the rule.**
- **C — decline.** Stay a client (the default path). Always available, zero risk,
  and sufficient for the requirement.

**Consequences for the spec.** The divergence constraint is scoped to the native
path only and must state reversibility; the client path gets its own requirement
asserting that it claims no exclusivity and that a permission prompt is
answerable from any device.

### 6.4 Open upstream asks

Three paseo observations, each independently filed in the change's task list:

1. `logs --json` accepts the flag and emits the same human text — no structured
   timeline feed for any client.
2. `inspect <full-agent-id>` returns an `Id` matching neither the agent id nor
   `persistence.nativeHandle`.
3. No release/park primitive, which is what blocks implementation A above.
