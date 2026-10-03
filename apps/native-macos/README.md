# T3 Code native macOS client

Experimental Rust/GPUI client for this fork. Development stays on
`t3code/port-t3code-to-rust-gpui`; it is not a replacement for the shipping app.

## Build

```sh
cargo check --manifest-path apps/native-macos/Cargo.toml
cargo run --manifest-path apps/native-macos/Cargo.toml
```

Requires macOS and Xcode developer tools. GPUI is pinned to 0.2.2 and uses its
supported runtime Metal shader compilation, so development does not require
Xcode's separately downloaded offline Metal toolchain. Signing, notarization
and release-performance validation remain tracked in the macOS workstream.

The current executable implements a read-only project/thread sidebar using the
existing server's authenticated Effect RPC shell subscription. It renders live
project/thread metadata and supports thread selection, disconnect and manual
retry. Conversation rendering, composer, thread actions, automatic reconnect,
pairing, relay/DPoP credentials and Keychain persistence are not implemented yet.
There is no embedded Electron renderer.

For development, set `T3_NATIVE_SERVER` to the HTTP(S) server origin and supply an
existing bearer **access token** through `T3_NATIVE_ACCESS_TOKEN`, then run the
app. Do not use a pairing-link token or put credentials in command-line arguments.
Remote origins require HTTPS; plain HTTP is accepted only for loopback. The app
exchanges the bearer credential for a one-time WebSocket ticket and includes the
current orchestration protocol version. It does not persist credentials.

The socket worker runs outside GPUI's UI thread with a bounded delivery channel.
It acknowledges accepted chunks, uses Effect's JSON heartbeat protocol, reports
subscription failures, and is cancelled when the UI task is dropped. Projection
tests cover snapshot loading, synchronization, replay deduplication, rename,
archive/unarchive and removal. Local fixture-server tests exercise the complete
bearer-to-ticket-to-WebSocket flow, authorization rejection and chunk acknowledgement;
they are not a substitute for a real-server/client parity run.

## Compatibility constraint

The existing Node server remains the authority for orchestration, durable
history, provider execution and checkpoints. Port the client, not those
systems, initially. The Rust transport must implement the existing Effect RPC
wire protocol and authorization flows; inventing a JSON-RPC lookalike would
silently break subscriptions, cancellation and remote authentication.

GPUI is a UI framework, not a browser engine. Browser profile import, preview,
recording, automation and element picking require an explicit browser adapter.
A simplified URL viewer cannot be accepted as parity with the Electron client.

## Evaluate

```sh
cargo test --manifest-path apps/native-macos/Cargo.toml
python3 -m unittest discover -s apps/native-macos/parity -p 'test_*.py'
python3 apps/native-macos/parity/evaluate.py evaluate
python3 apps/native-macos/parity/evaluate.py evaluate --evidence /path/to/evidence.json
```

Evaluation exits nonzero until every scenario has paired passing evidence,
every inventory unit has been reviewed, and the reference source has no drift.
`verified_scenario_percent` measures only the specified scenarios, **not** the
percentage of the entire product implemented.

`parity/baseline.json` pins the current fork revision, source/RPC discovery
inventory and initial behavioral scenarios. These are workstream-level discovery
scenarios, not an exhaustive test specification yet. Review every inventory unit
and expand scenarios into repeatable fixtures and action sequences before a
100% parity claim. Put inventory-to-scenario mappings in `coverage_review`:

```json
{
  "apps/web/src/example.ts": {"scenarios": ["threads-01"]},
  "apps/web/src/example.test.ts": {
    "non_behavioral_reason": "Reference tests; their behaviors are covered by threads-01"
  }
}
```

Evidence is keyed by scenario ID:

```json
{
  "threads-01": {
    "current": {
      "result": "pass",
      "revision": "<baseline git commit>",
      "artifact": "/absolute/path/to/current-run.json",
      "sha256": "<artifact SHA-256>"
    },
    "native": {
      "result": "pass",
      "revision": "<native git commit>",
      "source_sha256": "<native_source_sha256 from evaluation output>",
      "artifact": "/absolute/path/to/native-run.json",
      "sha256": "<artifact SHA-256>"
    }
  }
}
```

Artifacts must record fixtures, actions and observed results, including visible
behavior, durable state, commands, errors and relevant performance measurements.
Use identical inputs for both clients. Native evidence is invalidated when the
Rust source or Cargo manifest/lockfile changes. Reference evidence must match
the pinned fork revision.

The evaluator currently checks evidence integrity and coverage gates. It does
not execute client actions or independently judge artifact assertions; paired
behavioral runners and human review are still required. Do not convert a source
inventory or a successful build into a behavioral pass.

Tracking: [evaluation issue #44](https://github.com/ricotrevisan/t3code/issues/44).
Workstream issue URLs are in `parity/issues.json`.
