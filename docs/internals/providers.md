# Provider architecture

> For maintainers. Using T3 Code? See [docs/user](../user/).

A provider is the agent runtime that does the actual work. T3 Code supports several, and the
orchestration layer does not know which one is behind a thread.

## Built-in drivers

[`builtInDrivers.ts`][drivers] exports `BUILT_IN_DRIVERS` with six entries:

| Driver kind   | Driver source                           |
| ------------- | --------------------------------------- |
| `codex`       | [`Drivers/CodexDriver.ts`][codex]       |
| `claudeAgent` | [`Drivers/ClaudeDriver.ts`][claude]     |
| `cursor`      | [`Drivers/CursorDriver.ts`][cursor]     |
| `grok`        | [`Drivers/GrokDriver.ts`][grok]         |
| `opencode`    | [`Drivers/OpenCodeDriver.ts`][opencode] |
| `primeAgent`  | [`Drivers/PrimeDriver.ts`][prime]       |

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

Built-in drivers are compiled into the server. Trusted local adapters can instead be registered in
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
      "id": "pi-rpc",
      "version": "1.0.0",
      "driver": "piRpc",
      "modulePath": "/absolute/path/to/pi-rpc/index.mjs",
      "enabled": true
    }
  ]
}
```

An instance backed by that package pins the same identity in `ServerSettings.providerInstances`:

```json
{
  "providerInstances": {
    "pi_local": {
      "driver": "piRpc",
      "adapterPackage": {
        "id": "pi-rpc",
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
cannot replace a built-in driver. An external instance is materialized only when its pinned package
reference exactly matches the loaded package. Missing, invalid, mismatched, or incompatible packages
do not stop server startup; configured instances stay visible as unavailable with the package failure
reason and requested package identity.

External code is full-trust and executes only in the server process. It translates its native harness
protocol into canonical sessions, commands, and runtime events. It does not receive internal
`ProviderDriver` services. Packages that need a subprocess must use the V1 host process supervisor,
which owns process spawning, bounded stdio, termination, and scope finalizers. Every spawned process
declares whether it is a probe or session-owned; multiplexed processes can attach more sessions.
Unexpected exits of session-owned processes become canonical `runtime.error` events, so the host
clears the affected running turn even when package code does not observe `exitCode`. The host also
stamps package, provider, and instance identity onto package output, rejects negotiated capabilities
absent from the package manifest, and exposes the negotiated set as
`ServerProvider.adapterCapabilities`.

Web, desktop renderer, and mobile clients receive normalized snapshots/events, declarative
configuration schemas, and package identity, never `modulePath` or executable code. Each materialized instance runs in a registry-owned child
scope. The host continues to own routing, persistence, session reaping, orchestration, checkpoints,
and fallback text generation. Package activation currently requires a server restart; there is no
installer, update UI, remote package source, client plugin surface, or external text-generation hook.

Adding a built-in driver still means writing the driver plus adapter and adding it to
`BUILT_IN_DRIVERS`. Adding a trusted local package requires no T3 rebuild and targets only the
versioned `@t3tools/provider-adapter` interface. A real subprocess-driven echo package lives in
`apps/server/src/provider/testFixtures/` (`echoAdapterPackage.mjs` plus `echoHarness.mjs`) and its
conformance suite proves the seam end to end: process supervision, JSONL framing, canonical event
translation, interrupts, rollback, and graceful stops without crash events.

### Pi RPC reference adapter

`piAdapterPackage.mjs` in the same fixture directory exercises a real harness protocol through V1.
Its default process is `piHarness.mjs`, a deterministic fake; it is not an installed Pi provider.
An explicit `binaryPath` and `args` can target a real Pi CLI in `--mode rpc` mode. The protocol
reference is Pi commit `71dca871bc80b6bc97be37f0ca3189399d651fff`.

Pi's `turn_end` means one assistant response plus its tool calls, not a completed T3 turn. A T3 turn
must stay open through tool loops and retries until `agent_settled`. Prompt acknowledgement is not
completion: Pi can acknowledge before producing any events. Model identity must retain both the Pi
provider and model ID. Extension UI responses are notifications, not RPC requests with a second
acknowledgement. Display-only extension notifications must not become blocking user questions.

The fixture is a conformance reference, not a claim of complete Pi support. Capabilities not mapped
through V1 must stay unadvertised. Real-process smoke checks use a separate temporary agent home,
session directory, and workspace, with loopback inference instead of user credentials. Fake-harness
checks alone cannot establish compatibility with a released Pi build.

The reference adapter was smoke-tested through `ExternalProviderDriver` against
`@earendil-works/pi-coding-agent@0.85.1` (published git head
`d981de1229ef899957bbe968bc8dcda02a21f477`), using loopback model responses and real Pi tool execution.
That check covers provider-qualified model switching, multi-response tool loops, final usage,
active-branch history, native-session resume, and another prompt after resume. A controlled Pi
extension also verifies confirmation, empty input, display-only notifications, and commands that
finish without starting an agent run. This npm build and the protocol-document commit are distinct pins.

Current gaps include attachments, approvals, rollback, queued follow-up delivery, context-window
streaming, and complete extension-dialog timeout/cancellation synchronization. Display-only UI
updates are ignored. The structured-input mapping handles select, confirm, input, and editor answers;
it does not provide a general extension UI. Pi is still a reference fixture, not a shipped provider
installation flow.

Prime remains a built-in driver. Its RPC sessions and model/approval probes now acquire processes
through the same host-owned supervisor used by external packages. `PrimeRpcClient` receives the
supervised process and owns only JSONL framing, request correlation, event fanout, and Prime error
translation. The adapter preserves Prime-specific turn and session cleanup while the host owns
bounded stdin, spawn resolution, exit detection, termination, and scope finalizers.

Prime's filesystem resources, native resume validation, managed snapshots, and text-generation
policy are still host-side concerns; V1 does not expose the server's Effect services to packages. A
shared protocol/process seam must not bypass those requirements or be described as an independently
installable Prime package before that separation exists.

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
[codex]: ../../apps/server/src/provider/Drivers/CodexDriver.ts
[claude]: ../../apps/server/src/provider/Drivers/ClaudeDriver.ts
[cursor]: ../../apps/server/src/provider/Drivers/CursorDriver.ts
[grok]: ../../apps/server/src/provider/Drivers/GrokDriver.ts
[opencode]: ../../apps/server/src/provider/Drivers/OpenCodeDriver.ts
[prime]: ../../apps/server/src/provider/Drivers/PrimeDriver.ts
[adapter]: ../../apps/server/src/provider/Services/ProviderAdapter.ts
[instances]: ../../apps/server/src/provider/Services/ProviderInstanceRegistry.ts
[registry]: ../../apps/server/src/provider/Services/ProviderAdapterRegistry.ts
[service]: ../../apps/server/src/provider/Layers/ProviderService.ts
[contracts]: ../../packages/contracts/src/orchestration.ts
[worker]: ../../packages/shared/src/DrainableWorker.ts
[ingest]: ../../apps/server/src/orchestration/Layers/ProviderRuntimeIngestion.ts
[cmd]: ../../apps/server/src/orchestration/Layers/ProviderCommandReactor.ts
[checkpoint]: ../../apps/server/src/orchestration/Layers/CheckpointReactor.ts
[external]: ../../apps/server/src/provider/TrustedLocalProviderAdapters.ts
