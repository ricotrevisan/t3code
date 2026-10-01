# Pi preview startup repair (release v0.0.48)

## Confirmed failure chain

The live release source map's `PiAdapter.ts` exactly matches commit
`1b5c30e4ecfc67dcda6238fc1a09468737b173ec`. Changes are based on that release,
not the newer sync branch, which no longer contains the same Pi package.

1. Installed Pi **0.99.2** awaits `session.bindExtensions()` (including extension
   `session_start` UI consent) before attaching its RPC stdin reader. A synthetic
   startup confirmation emitted a UI request, withheld `get_commands`, and could
   not consume an explicit denial. With no other active handles Pi even exits;
   the keepalive fixture models an extension maintaining a live connection.
2. T3 checks `get_commands` before adding a Pi session to its ready-session map.
   Startup replies consequently fail with `No Pi session for thread 'startup'.`
3. ProviderService persists a binding only after native startup returns.
   Startup replies consequently fail with `Cannot route thread ... because no
   persisted provider binding exists.`
4. ProviderCommandReactor uses one serial command lane; a UI response queues behind
   the startup handshake that is itself waiting for that response.
5. The 30-second Pi RPC deadline expires while consent is pending and its error is
   collapsed into `T3 preview bridge did not initialize. Restart the Pi session to retry.`

The real Pi fixture uses synthetic consent and a synthetic health command, never
project configuration, project MCP services, real credentials, or model inference.
The T3 tests exercise the actual adapter, service, and reactor at their respective
process/registry boundaries. They are not a production UI end-to-end verification.

## T3 repair

- Startup-only Pi transport map routes extension UI replies without claiming the
  session is ready. It is removed on success, failure, and interruption.
- Startup-only ProviderService map routes UI replies to the exact adapter during
  fresh startup and recovery; it lasts through binding persistence. Concurrent
  starts fail before replacing MCP credentials. No fictional binding is inserted.
- Only UI responses for a projected `starting` session use a separate drainable
  lane; normal session/turn commands keep their ordered lane.
- RPC waits retain the existing deadline when no UI request is outstanding. While
  consent is pending, the deadline is rearmed rather than failing the session.
  Bridge acknowledgement is still required after consent, including denial.

## Pi transport prerequisite — REQUIRED

The T3 changes alone cannot make Pi 0.99.2 consume a startup response.
`pi-0.99.2-startup-input.patch` shows the separately tested Pi runtime change:
attach stdin and shutdown signal handlers before binding extensions, handle UI
responses immediately, and gate ordinary commands until initialization completes.

`prepare-pi-runtime.py SOURCE DESTINATION` copies a pinned Pi 0.99.2 package to a
**new isolated directory**, verifies the exact original RPC source hash, and applies
that change to the copy. It never edits the source installation.

**Use `node DESTINATION/dist/cli.js`, not the globally installed wrapper and not
`dist/bundle/cli.js`.** The bundled entry does not use the patched unbundled module.
An upstream Pi source fix and rebuilt bundle is the preferable long-term route.

## Reproduction and verification

From the repair worktree:

```sh
pnpm --filter @t3tools/provider-adapter-pi exec vp test run --testTimeout 5000
cd apps/server
pnpm exec vp test run src/provider/Layers/ProviderService.test.ts src/orchestration/Layers/ProviderCommandReactor.test.ts src/provider/PiAdapterConformance.test.ts --testTimeout 15000
cd ../..
pnpm --filter @t3tools/provider-adapter-pi typecheck
pnpm --filter t3 typecheck
```

Before fixes, the service regression failed in 22ms with the missing-binding error;
Pi immediate consent failed with the missing-session error, and 61 simulated seconds
closed startup prematurely. Reactor delivery timed out after 500ms. These were
independently run red before changing their corresponding behavior.

Real Pi prerequisite repro (run fixtures from a scratch directory, not a project):

```sh
# Copy real-pi-startup-repro.py and startup-consent-fixture.mjs to scratch.
# Unpatched installed Pi: expected assertion failure after denial cannot be read.
python3 /path/to/scratch/real-pi-startup-repro.py
# Prepare a NEW isolated runtime, then verify the repaired unbundled entry.
python3 scripts/debug/pi-startup/prepare-pi-runtime.py /path/to/pi-package /path/to/new-runtime
PI_REPRO_CLI=/path/to/new-runtime/dist/cli.js python3 /path/to/scratch/real-pi-startup-repro.py
```

Verified repaired output:

```text
REAL_PI_STARTUP_UI_REQUEST: confirm
GET_COMMANDS_BLOCKED_BEFORE_APPROVAL: yes
GET_COMMANDS_AFTER_DENIAL: success; t3-preview-health present
```

## Rollout (not performed)

1. Review/cherry-pick this release-based T3 change. Re-run the commands above.
2. Prepare/verify a patched Pi runtime or consume an upstream Pi release that fixes
   this initialization ordering. Point only the intended Pi provider instance at
   `node` plus the repaired unbundled CLI path, retaining its existing args/env.
   This is an explicit separate prerequisite, not an automatic global Pi patch.
3. Build a new T3 release/server bundle and stage it beside the current release.
   Switching the release and restarting the server requires operator approval.
4. Start a fresh/retried Pi session. Verify consent remains answerable beyond 30s,
   both Allow and Deny finish startup, the preview health command is present, and
   a real ready binding is persisted. Verify unrelated providers and preview actions.
5. Existing dead-process approval cards cannot revive their original request. Do not
   insert runtime bindings manually or reuse stale request IDs; retry startup.
   Any UI cleanup of historical pending cards is a separate operation.
6. Roll back both the T3 release selection and Pi instance binary/args if needed.

No production restart, deployment, database write, settings change, global Pi
package write, or Bubble plugin edit was made while preparing this repair.
