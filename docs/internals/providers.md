# Provider constraints

Orchestration records intent and state without knowing which provider runs a thread. Provider
protocols, account ownership, permissions, and capabilities belong at the
[adapter boundary](../../apps/server/src/provider/Services/ProviderAdapter.ts). Normalize there
instead of spreading provider checks through reactors and clients.

A driver kind identifies an integration; an instance identifies one configuration and account
lifecycle. Route work by instance, so two accounts using the same driver do not share mutable
session or catalog state.

## Process and account isolation

T3-managed OpenCode chat uses one server per thread. Its MCP registrations are directory-scoped, while
T3's MCP connection is thread-scoped. Sharing a chat server between threads in one directory would
let them replace each other's connection. Catalog and text-generation work can share the
[instance-owned helper](../../apps/server/src/provider/OpenCodeServerOwner.ts), which closes
after an idle period. External OpenCode servers remain externally owned and can require an
external restart to pick up configuration changes.

OpenCode also stores persistent approval grants per directory. Automatic full-access replies use
`once` so they cannot widen a supervised thread's permissions on a shared external server.
See the [adapter](../../apps/server/src/provider/Layers/OpenCodeAdapter.ts).

Antigravity separates account profiles per instance while sharing installed executables across the
environment. It forces file-based credential storage because the native macOS keychain entry would
otherwise be shared across instances. The launch environment removes ambient Google credentials,
so an instance cannot silently use another account or billing project. The agent also resolves
its user-global skill directories under that profile, so the profile links those two directories
back to the user's real `~/.gemini`; MCP servers, hooks, and rules there stay out of the profile.
See [profile isolation](../../apps/server/src/provider/antigravityAuthSupport.ts).

The [Antigravity installer](../../apps/server/src/provider/AntigravityInstallation.ts) outlives
client connections and provider-instance rebuilds. Releases are immutable, with an atomic pointer
selecting the version for new processes. Running processes hold leases on their version. Updates
and removal must respect those leases instead of replacing executables under a running agent.

## Setup must not happen as a health-check side effect

Opening a provider session can start MCP servers, run hooks, or launch a login browser.
[Grok probes](../../apps/server/src/provider/Layers/GrokProvider.ts) avoid authentication and
session creation for this reason. Antigravity likewise reserves authenticated catalog sessions for
explicit setup or model refresh; background checks use initialization only.

[Antigravity sign-in](../../apps/server/src/provider/AntigravityAuth.ts) belongs to the initiating
T3 auth session. The client carries the return URL back to the environment because the provider's
loopback listener may be on another machine. Forward only the callback for the owned pending flow;
a successful callback HTTP request is not proof that provider authentication finished. The native
process owns token exchange and storage.

Antigravity sign-out closes admission to new processes and stops existing processes before clearing account
metadata. Otherwise a helper or resumed session could retain the old account. Cached model lists
do not establish current access, and an authoritative empty catalog must clear the old list.

Antigravity text-generation helpers deny tool requests, but native hooks and MCP configuration can
run before the prompt. They reject profiles with such configuration before launch. Prompt
instructions and tool denial do not create a native sandbox.
See [helper constraints](../../apps/server/src/textGeneration/AntigravityTextGeneration.ts).

## Provider updates run only through the owning installer

A one-click update is offered only when the resolved executable's path proves which installer owns
it. Homebrew and npm are proven by the real path (symlinks followed): a versioned keg or cask under
`brew --prefix`, or `<prefix>/lib/node_modules/<pkg>/` (Windows: the shim beside `node_modules`).
Native installer layouts and the global bin directories of pnpm, Bun, and Vite+ may match on either
the resolved path or its real target, since those installers place real files or their own symlinks
there. Anything unproven stays manual-only but still reports the version gap. npm updates pin
`--prefix` because the `npm` on `PATH` can belong to a different Node than the one that owns the
provider. Homebrew
compares against `brew info` since casks trail npm by hours; native installs share npm's version
train, so the registry stays authoritative for them.
See the [resolver](../../apps/server/src/provider/providerMaintenance.ts).

Ownership is cached per instance and re-read immediately before an update runs. The
[runner](../../apps/server/src/provider/providerMaintenanceRunner.ts) refuses when the lock key
changed since the advisory, and reports success only when the refreshed provider is still installed
with a readable, current version.

## Protocol traps

Codex async questions arrive as notifications and are answered with a new user message. There is
no pending RPC response to send. Blocking questions still use the request/response path. The
[adapter](../../apps/server/src/provider/Layers/CodexAdapter.ts) distinguishes them; the
[decider](../../apps/server/src/orchestration/decider.ts) records an async answer and its user
message together.

An async question can outlive the turn or a server restart. The engine reads that request's
durable activity before resolving it because the in-memory command snapshot omits old activities.
Do not infer that a request has disappeared merely because it is outside the recent window.

Capabilities must describe what the provider can actually do. Antigravity can capture workspace
checkpoints but cannot roll back its conversation. The [checkpoint boundary](./overview.md#turn-completion-and-checkpoints)
therefore rejects revert before touching files. Native permission and question option IDs must
also survive normalization; a display label is not necessarily a valid reply.

## Attachments and stored history

Attachments live outside the project workspace. [ProviderService](../../apps/server/src/provider/Layers/ProviderService.ts)
puts their environment-local paths in turn input and lets adapters choose native input formats.
A path in the prompt does not grant filesystem access. Keep provider sandbox and approval rules
in force; copying uploads into the project to bypass them changes that boundary.

File attachments introduced a replay compatibility limit. Image-only clients cannot decode
file-bearing messages, and an image-only server can fail the entire environment's startup when
replaying one such event. Rollouts and downgrades must account for persisted history as well as
current client support.

Model classification has its own [manifest constraints](./model-manifest.md). Assistant-reference
handling is documented under [citations](./assistant-citations.md).

# Provider architecture

> For maintainers. Using T3 Code? See [docs/user](../user/).

A provider is the agent runtime that does the actual work. T3 Code supports several, and the
orchestration layer does not know which one is behind a thread.

## Built-in drivers

[`builtInDrivers.ts`][drivers] exports `BUILT_IN_DRIVERS` with five unversioned core entries:

| Driver kind   | Driver source                           |
| ------------- | --------------------------------------- |
| `codex`       | [`Drivers/CodexDriver.ts`][codex]       |
| `claudeAgent` | [`Drivers/ClaudeDriver.ts`][claude]     |
| `cursor`      | [`Drivers/CursorDriver.ts`][cursor]     |
| `grok`        | [`Drivers/GrokDriver.ts`][grok]         |
| `opencode`    | [`Drivers/OpenCodeDriver.ts`][opencode] |

[`FirstPartyProviderAdapters.ts`][first-party] registers the compiled workspace packages for Prime
(`prime-rpc@1.0.0`), Pi (`pi-rpc@1.0.0`), and DeepSeek Harness. All run through
`makeExternalProviderDriver`, with the same package identity, host negotiation, output decoding,
defect isolation, and crash handling as trusted-local packages. Prime's implementation lives in
[`@t3tools/provider-adapter-prime`][prime] and depends only on the public adapter SDK plus shared
contracts and helpers; it no longer uses a private compiled driver bridge or server Effect services.

Each driver declares its `driverKind`, a `configSchema`, and a `create` function that builds an
adapter in a child scope. Adapter implementations live beside them in
`apps/server/src/provider/Layers/` (`CodexAdapter.ts`, `ClaudeAdapter.ts`, and so on) and conform to
[`ProviderAdapter.ts`][adapter]. Read the driver plus its adapter to see how a specific agent's
transport, config, and event shapes are mapped.

## Registry and routing

Two registries separate configuration from live processes:

- [`ProviderInstanceRegistry`][instances] keys configured instances by `ProviderInstanceId`. Creating
  one looks up the driver by `driverKind`, decodes `entry.config` with that driver's schema, opens a
  child scope, and calls `driver.create`.
- [`ProviderAdapterRegistry`][registry] resolves an instance ID to its live adapter via
  `getByInstance`.

[`ProviderService`][service] sits on top. It combines the adapter registry with the provider session
directory to route session and turn operations for a thread, so callers name a thread, not an agent.

`ProviderService.sendTurn` expands [assistant citations](./assistant-citations.md) into quoted
reference data before dispatching to any adapter. Bound user comments remain distinct from the quoted
assistant text. Persisted messages keep their serialized links.

The server reaps inactive provider sessions while preserving their persisted resume state. The
inactivity threshold is 30 minutes. `lastSeenAt` advances when a turn is sent and when the session
emits turn, task, or user-input lifecycle events, so a long-running turn is not treated as idle
from the user prompt. Active turns, pending user input, and reported background work (subagents,
monitor loops, Prime heartbeat jobs) are never reaped. A later turn starts the provider again from the saved cursor.

Prime Agent can start a new cycle after T3 has already settled the user turn
(scheduled heartbeats, late child reports). The Prime adapter opens a new T3 turn
on that `agent_start` and publishes active heartbeat cron jobs as `monitor`
tasks so the reaper treats them as live background work.

Built-in drivers and first-party adapter packages are compiled into the server. Trusted local
adapters can instead be registered in
`<T3 state dir>/provider-adapters.json` and loaded at server startup. This file is deliberately
separate from `ServerSettings.providerInstances`:

- an **adapter package registration** identifies installed server code (`id`, `version`, `driver`,
  and `modulePath`);
- a **provider instance** selects that driver and supplies display, environment, and adapter config;
- a **live session** belongs to one instance and one T3 thread.

The registry file has this shape:

```json
{
  "schemaVersion": 1,
  "packages": [
    {
      "id": "my-local-agent",
      "version": "1.0.0",
      "driver": "myLocalAgent",
      "modulePath": "/absolute/path/to/my-local-agent/index.mjs",
      "enabled": true
    }
  ]
}
```

An instance backed by that package pins the same identity in `ServerSettings.providerInstances`:

```json
{
  "providerInstances": {
    "my_local_agent": {
      "driver": "myLocalAgent",
      "adapterPackage": {
        "id": "my-local-agent",
        "version": "1.0.0",
        "protocolVersion": 1
      },
      "config": {}
    }
  }
}
```

The module default export is a `ProviderAdapterPackageV1` from the workspace SDK
`@t3tools/provider-adapter`. It contains a declarative manifest, a runtime configuration codec,
default configuration, and an instance factory. The manifest pins package identity, host protocol
compatibility, supervised stdio protocol and session concurrency, JSON-shaped configuration schema,
and the maximum capabilities the package may negotiate. [`TrustedLocalProviderAdapters.ts`][external]
validates the registration, module, and compatibility before wrapping it as a host driver. A package
cannot replace a built-in or compiled first-party driver. An external instance is materialized only
when its pinned package
reference exactly matches the loaded package. Missing, invalid, mismatched, or incompatible packages
do not stop server startup; configured instances stay visible as unavailable with the package failure
reason and requested package identity.

External code is full-trust and executes only in the server process. It translates its native harness
protocol into canonical sessions, commands, and runtime events. It does not receive internal
`ProviderDriver` services. Host protocol V1 supplies the process supervisor, which owns spawning,
bounded stdio, termination, and scope finalizers. Host protocol V2 adds narrow host-owned workspace
resolution, package/session storage, contained artifact materialization, resume-file validation, and
attachment reads. [`ExternalProviderAdapterHost.ts`][external-host] implements those resources
without exposing `ServerConfig`, filesystem services, paths, or arbitrary Effect Context to packages.
The server negotiates the highest supported host version in the manifest range and still gives an
exact V1 shape to older packages.

Every spawned process declares whether it is a probe or session-owned; multiplexed processes can
attach more sessions. Unexpected exits of session-owned processes become canonical `runtime.error`
events, so the host clears the affected running turn even when package code does not observe
`exitCode`. An adapter that already translates its native shutdown into canonical terminal events can
detach that session from generic crash projection to avoid duplicate terminal sources. The host also
stamps package, provider, and instance identity onto package output, rejects negotiated capabilities
absent from the package manifest, and exposes the negotiated set as
`ServerProvider.adapterCapabilities`.

The exact package ID, version, and protocol are also persisted as one optional structured identity on
provider runtime bindings and projected thread sessions. Migration 55 adds three nullable columns to
each table. Readers accept either a complete identity or no identity and reject partial triples. On
canonical session and runtime-event contracts, an omitted field means legacy/unknown, `null` is an
authoritative package-less adapter, and an object is the exact package identity. This distinction lets
package-less rebinds clear stale projected identities without rewriting old events. The legacy
`adapter_key` remains dual-written for downgrade and recovery compatibility; a successful legacy
resume upgrades the binding to the loaded package identity.

Web, desktop renderer, and mobile clients receive normalized snapshots/events, declarative
configuration schemas, and package identity, never `modulePath` or executable code. The optional
`ServerConfig.providerAdapterManifests` catalog combines validated compiled manifests with manifests
from successfully loaded trusted packages. It deterministically sorts and deduplicates exact package
identities. Startup rejects trusted-package driver collisions and exact identity collisions with
compiled packages rather than letting local code shadow them.

The web client and desktop renderer use a small schema subset for adapter settings: strings, booleans,
numbers, integers, string enums, and string arrays. They preserve fields or schemas they cannot edit.
A configured instance is editable only when its driver and exact package ID, version, and protocol
match a live provider or catalog manifest. A stale instance stays visible and deletable, but its
configuration is read-only until that exact package is installed again. Mobile administration for
these package-backed instances is deferred.

Each materialized instance runs in a registry-owned child scope. The host continues to own routing,
persistence, session reaping, orchestration, checkpoints, and fallback text generation. Package
activation currently requires a server restart; there is no installer, update UI, remote package
source, client plugin surface, or external text-generation hook.

Adding an unversioned built-in driver still means writing the driver plus adapter and adding it to
`BUILT_IN_DRIVERS`. No orchestration, contract, or client change is required for the common case.
A compiled first-party registration instead belongs in `FirstPartyProviderAdapters.ts` and must carry
a package identity, declarative configuration schema, and a declared capability ceiling. Adding a
trusted local package requires no T3 rebuild and targets only the versioned
`@t3tools/provider-adapter` interface. A real subprocess-driven echo package lives in
`apps/server/src/provider/testFixtures/` (`echoAdapterPackage.mjs` plus `echoHarness.mjs`) and its
conformance suite proves the seam end to end: process supervision, JSONL framing, canonical event
translation, interrupts, rollback, and graceful stops without crash events.

### ACP runtime and DeepSeek Harness

[`@t3tools/provider-adapter-acp`][acp-package] is the generic package runtime for ACP v1 agents. It
runs one host-supervised stdio process and multiplexes new and resumed sessions over its JSON-RPC
connection. T3 owns spawning, environment injection, bounded stdio, exit detection, termination,
and scope cleanup; the runtime owns ACP initialization, request correlation, session attachment, and
translation.

[`AcpEventMapper.ts`][acp-mapper] maps standard ACP updates to canonical provider events. Agent
message and thought chunks become assistant and reasoning item lifecycles; tool calls become typed
tool lifecycle items; plans become `turn.plan.updated`; available commands, current mode, and config
options become `session.configured`; session information becomes `thread.metadata.updated`; and
usage becomes `thread.token-usage.updated`. User message echoes are ignored because the prompt is
already in the canonical timeline. Every emitted event carries the provider, instance, thread, turn
when applicable, and exact adapter package identity.

The runtime advertises only features that are both implemented by the runtime and declared by the
package manifest. A manifest that requires `session.resume` makes missing ACP resume support an
`initialize` failure; a package that does not declare resume cannot use it even if the agent offers
it. Local `readThread` history is sanitized and bounded separately from the lossless live event
stream: at most 100 turns and 200 items per turn, with bounded strings, collections, and nesting.
Attachments, plan interaction mode, conversation rollback, and structured input are rejected and
remain outside the advertised capability set. Incoming ACP plan updates are still projected as
canonical plan events.

[`DeepSeekHarnessAdapter.ts`][deepseek] compiles the first shipped package on this runtime as
`deepseek-harness-acp@1.0.0`, with driver `deepseekHarness`. Its default launch is
`dsh --profile acp`; configuration can override the command, arguments, and working directory, while
secrets remain provider-instance environment variables. Compatibility was validated against the
real npm release `@deepseek-ai/dsh@0.1.5-rc.2`, built from git commit
`c291e7961a515f6d7af9304e7fd1d257929aef26`, in an isolated keyless smoke test.

### Pi RPC adapter

[`@t3tools/provider-adapter-pi`][pi-package] is the shipped `pi-rpc@1.0.0` package, with driver
`piRpc`. It starts one host-supervised Pi process per T3 session in `--mode rpc`. The manifest
requires Host V2: T3 resolves the authorized workspace, assigns an instance- and thread-contained
session directory, forces Pi to write there, and validates a native session file before resume. The
package config exposes the executable, additional launch arguments, optional default working directory,
model, and thinking level. The adapter owns `--mode rpc` and the Host V2 session-directory argument;
the shipped executable default is `pi`.

Pi's `turn_end` means one assistant response plus its tool calls, not a completed T3 turn. A T3 turn
must stay open through tool loops and retries until `agent_settled`. Prompt acknowledgement is not
completion: Pi can acknowledge before producing any events. Model identity retains both the Pi
provider and model ID. Extension UI responses are notifications, not RPC requests with a second
acknowledgement. Display-only extension notifications do not become blocking user questions.

The deterministic `piHarness.mjs` fixture exercises the package through `ExternalProviderDriver`,
including successful native resume after an adapter/process restart and another prompt on the resumed
session. Real-process smoke checks use a separate temporary agent home, host-owned session directory,
and workspace, with loopback inference instead of user credentials. Fake-harness checks alone do not
establish compatibility with a released Pi build.

The adapter was smoke-tested against `@earendil-works/pi-coding-agent@0.85.1` (published git head
`d981de1229ef899957bbe968bc8dcda02a21f477`), using loopback model responses and real Pi tool execution.
That check covers provider-qualified model switching, multi-response tool loops, final usage,
active-branch history, native-session resume, and another prompt after resume. A controlled Pi
extension also verifies confirmation, empty input, display-only notifications, and commands that
finish without starting an agent run. This npm build and the protocol-document commit
`71dca871bc80b6bc97be37f0ca3189399d651fff` are distinct pins.

Until a controlled Pi permission extension is wired through the package, snapshots advertise only the
`full-access` runtime mode and session start rejects other modes. Current gaps include attachments,
approvals, rollback, queued follow-up delivery, context-window
streaming, and complete extension-dialog timeout/cancellation synchronization. Display-only UI
updates are ignored. The structured-input mapping handles select, confirm, input, and editor answers;
it does not provide a general extension UI. Capabilities that are not mapped through protocol V1 stay
unadvertised.

Prime is no longer in `BUILT_IN_DRIVERS` and has no server-private `PrimeDriver`. The compiled
`@t3tools/provider-adapter-prime` workspace package registers as `prime-rpc@1.0.0` through the same
external driver bridge as ACP packages. The server reserves its `primeAgent` driver and exact package
identity against trusted-local replacement, and pins legacy or explicitly unpinned Prime instance
configs during hydration. An explicit stale package reference is never rewritten; normal registry
matching leaves that instance visible but unavailable.

Prime requires Host V2. Its RPC sessions and version/model/approval probes use host-supervised
processes. Session directories, extension artifacts, resume containment, workspace resolution, and
attachment bytes come from the host resource broker. The package owns JSONL framing, request
correlation, Prime protocol translation, canonical terminal events, and package snapshots. The
external bridge validates and stamps sessions, snapshots, events, package identity, config schema,
and negotiated capabilities before they enter core systems.

The workspace SDK and Prime package are still private source packages linked into this build; they are
not yet published or independently packable artifacts. That distribution constraint is separate from
the runtime deletion test: Prime itself now crosses `ProviderAdapterPackageV1.create` and the public
Host V2 boundary, with no Prime-specific driver registration or private server service access.

### Grok health check

`checkGrokProviderStatus` never opens an ACP session. It runs `grok --version`, then `grok models`
for login state and model slugs, then a single ACP `initialize` and reads models from
`_meta.modelState`. `authenticate` and `session/new` are skipped on purpose: `authenticate` can open
a browser login and `session/new` boots every configured MCP server, both of which made background
probes hang or surprise the user. A failed `initialize` degrades to `warning` with the CLI's model
list instead of persisting `error` over a working install. The built-in `grok-build` slug is the
CLI's product name, not an ACP model id. `applyGrokAcpModelSelection` treats it as "keep the
session's current model" and never sends it in `session/set_model`.

## OpenCode server ownership and catalog

Each OpenCode provider instance owns one lazy local server for catalog discovery and
text-generation helpers through [`OpenCodeServerOwner.ts`][opencode-server-owner]. Concurrent
borrowers share startup. The server closes 30 seconds after the last borrower releases it, or
when the provider instance closes. A failed or exited process can be started again on the next
use. An externally configured OpenCode server remains externally owned.

The local server and its SDK clients use one resolved password. An explicit provider password
overrides `OPENCODE_SERVER_PASSWORD` in the spawned environment. Without an explicit password,
the client uses the password from the environment that the process inherits. External servers use
only their explicit provider password and never inherit the host's local password.

Every server connection must pass the authenticated `/global/health` check before inventory or
session operations start. The response must contain a valid version at or above 1.14.19. Local
owners cache this result for the lifetime of the spawned process. External actions check once when
they create their server connection, not for each model or SDK request.

Chat adapters keep their own server per thread. They register a thread-specific `t3-code` MCP
connection, while OpenCode stores MCP connections by directory. Sharing these chat servers
without changing MCP routing would let two threads in one directory replace each other's
connection.

OpenCode loads its catalog through the HTTP API when an enabled provider instance starts. The
provider registry keeps the snapshot in memory and persists it in the existing per-instance cache.
Each `subscribeServerConfig` connection refreshes all providers, so a client reconnect reloads the
OpenCode catalog from the current helper. The `serverRefreshProviders` request also refreshes it.
Periodic OpenCode probes remain disabled. OpenCode reads credentials for each inventory request,
but its native configuration files can remain cached for the lifetime of the helper process. The
helper closes 30 seconds after its last inventory or text-generation borrower releases it. A
refresh after that idle period starts a new helper and reads file changes. Repeated refreshes and
active text-generation work can extend process reuse. Changes to the provider configuration or
environment replace the instance and start a new discovery. Changes to unrelated settings only
update snapshot enrichment. Other providers retain their existing refresh policy.

T3 Code does not own an external OpenCode process. Native configuration changes there can require
an external reload or restart before T3 Code's next refresh sees them.

The shared server's idle shutdown does not clear the catalog. Failed discovery keeps the last
known models, slash commands, and skills through the registry's existing merge rules. A successful
empty inventory is authoritative. Existing threads keep their explicit model identifier and
options when catalog metadata is missing; the catalog is not permission to choose a different
model for a thread.

## Model manifest

The model picker's legacy section is driven by `apps/server/src/provider/model-manifest.json`, which
lists the current (non-legacy) model slugs per driver kind. The `ModelManifest` service
(`apps/server/src/provider/ModelManifest.ts`) refreshes that data from the same file on `main` via
raw.githubusercontent.com, so moving a model in or out of the legacy section is a commit, not a
release. Preference order is remote fetch, then the on-disk copy of the last successful fetch (in
the state directory), then the bundled copy. Fetches are TTL-gated, run concurrently with provider
probes, respect the `enableProviderUpdateChecks` setting, and never fail a provider check. The
Codex and Claude drivers apply the classification to every snapshot with `applyModelManifest`;
driver kinds absent from the manifest have no legacy concept.

## Attachment access

The server stores uploaded attachments in its attachment directory, outside the project workspace.
`ProviderService` adds the absolute path of each attachment to the turn text, then passes every
attachment to the provider adapter. Each adapter decides what its provider ingests natively:

- Codex, Claude, Cursor, and Grok send images as native image inputs and skip generic files. For
  these providers, generic files reach the agent only as file paths in the turn text.
- OpenCode sends PNG/JPEG/GIF/WebP images, text files, and PDFs up to 20 MB as native file parts
  with their real mime type. Everything else (ZIP and other binaries, image formats model APIs
  reject, oversized files) falls back to the file path in the turn text, like the other providers.

Claude receives the attachment directory as an allowed additional directory. Codex keeps its
configured sandbox policy, so access depends on that policy and the selected runtime mode. OpenCode
allows all paths in full-access mode and requests approval for directories outside the workspace in
restricted modes. Cursor and Grok use their own provider permission rules.

The server does not copy attachments into a project or bypass provider approval rules. If an agent
cannot read an attachment, the user must approve the access or select a runtime mode that permits it.

Updated attachment schemas tolerate unknown attachment members, but old image-only clients still
cannot decode messages that contain file attachments. Client file-picking rollouts must account for
this limit.

Do not run an old image-only server against state that contains file attachments. Replay decodes
each persisted event before projection. A file-bearing event can make `ProjectionPipeline` bootstrap
and `OrchestrationEngine` startup fail for the entire environment, not only the affected thread.

## How provider work is requested

Clients never call a provider directly. They dispatch orchestration commands over the RPC method
`orchestration.dispatchCommand`, defined with the rest of the orchestration surface in
[`orchestration.ts`][contracts]. The client-dispatchable provider-facing commands are
`thread.turn.start`, `thread.turn.interrupt`, `thread.approval.respond`,
`thread.user-input.respond`, `thread.checkpoint.revert`, and `thread.session.stop`, plus the mode
setters `thread.runtime-mode.set` and `thread.interaction-mode.set`.

The engine persists an event for the command, and a server-side reactor performs the provider call.
Provider output comes back as internal commands such as `thread.message.assistant.delta` and
`thread.session.set`, which clients observe through `orchestration.subscribeThread`. See
[overview.md](./overview.md) for the command/event loop.

## Server-side workers

Provider work flows through three queue-backed workers. All three are built with
`makeDrainableWorker` from [`DrainableWorker.ts`][worker] and expose `drain` for deterministic test
synchronization.

1. [`ProviderRuntimeIngestion`][ingest] consumes provider runtime streams and emits orchestration
   commands.
2. [`ProviderCommandReactor`][cmd] reacts to orchestration intent events and dispatches provider
   calls.
3. [`CheckpointReactor`][checkpoint] captures workspace checkpoints on turn start and completion, and
   performs reverts.

### Buffered assistant delivery

A thread in `buffered` assistant delivery mode accumulates assistant text instead of streaming each
delta. The buffer is not held until turn completion. In [`ProviderRuntimeIngestion`][ingest],
`MAX_BUFFERED_ASSISTANT_CHARS` is 24,000: the append that would exceed it invalidates the buffer and
spills the whole accumulated text as one delta. The buffer also flushes at interaction boundaries,
when a request opens (approval) or user input is requested, via
`flushBufferedAssistantMessagesForTurn`.

[drivers]: ../../apps/server/src/provider/builtInDrivers.ts
[first-party]: ../../apps/server/src/provider/FirstPartyProviderAdapters.ts
[codex]: ../../apps/server/src/provider/Drivers/CodexDriver.ts
[claude]: ../../apps/server/src/provider/Drivers/ClaudeDriver.ts
[cursor]: ../../apps/server/src/provider/Drivers/CursorDriver.ts
[grok]: ../../apps/server/src/provider/Drivers/GrokDriver.ts
[opencode]: ../../apps/server/src/provider/Drivers/OpenCodeDriver.ts
[opencode-server-owner]: ../../apps/server/src/provider/OpenCodeServerOwner.ts
[prime]: ../../packages/provider-adapter-prime/src/index.ts
[pi-package]: ../../packages/provider-adapter-pi/src/index.ts
[adapter]: ../../apps/server/src/provider/Services/ProviderAdapter.ts
[external-host]: ../../apps/server/src/provider/ExternalProviderAdapterHost.ts
[instances]: ../../apps/server/src/provider/Services/ProviderInstanceRegistry.ts
[registry]: ../../apps/server/src/provider/Services/ProviderAdapterRegistry.ts
[service]: ../../apps/server/src/provider/Layers/ProviderService.ts
[contracts]: ../../packages/contracts/src/orchestration.ts
[worker]: ../../packages/shared/src/DrainableWorker.ts
[ingest]: ../../apps/server/src/orchestration/Layers/ProviderRuntimeIngestion.ts
[cmd]: ../../apps/server/src/orchestration/Layers/ProviderCommandReactor.ts
[checkpoint]: ../../apps/server/src/orchestration/Layers/CheckpointReactor.ts
[external]: ../../apps/server/src/provider/TrustedLocalProviderAdapters.ts
[manifest-catalog]: ../../apps/server/src/provider/ProviderAdapterManifestCatalog.ts
[acp-package]: ../../packages/provider-adapter-acp/src/index.ts
[acp-mapper]: ../../packages/provider-adapter-acp/src/AcpEventMapper.ts
[deepseek]: ../../apps/server/src/provider/DeepSeekHarnessAdapter.ts
