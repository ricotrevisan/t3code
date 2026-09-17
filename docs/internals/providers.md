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
provider runtime bindings and projected thread sessions. Migration 42 adds three nullable columns to
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
`BUILT_IN_DRIVERS`. A compiled first-party registration instead belongs in
`FirstPartyProviderAdapters.ts` and must carry a package identity, declarative configuration schema,
and a declared capability ceiling. Adding a trusted local package requires no T3 rebuild and targets
only the versioned `@t3tools/provider-adapter` interface. A real subprocess-driven echo package
lives in
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
