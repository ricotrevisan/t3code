# T3 Code native macOS client

Experimental Rust/GPUI client for this maintained fork. Development stays on
`t3code/port-t3code-to-rust-gpui`; it is not a replacement for the shipping app.

## Build and run

Requires macOS and Xcode developer tools. GPUI is pinned to 0.2.2, with compatible
GPUI Component 0.5.1 controls. Runtime Metal shader compilation avoids requiring
Xcode's separately downloaded offline Metal toolchain.

```sh
cargo build --manifest-path apps/native-macos/Cargo.toml
cargo run --manifest-path apps/native-macos/Cargo.toml
```

For development, set `T3_NATIVE_SERVER` to the HTTP(S) server origin and supply an
existing bearer **access token** through `T3_NATIVE_ACCESS_TOKEN`. Do not use a
pairing-link token or put credentials in command-line arguments. The app exchanges
the bearer credential for a one-time WebSocket ticket. Plain HTTP is accepted
only for loopback; remote origins require HTTPS. Credentials are not persisted.

Native drafts and stashed prompts use a separate data directory:
`~/Library/Application Support/T3 Code Native`. `T3_NATIVE_HOME` overrides this
for isolated testing. Corrupt or newer-version storage is preserved, not reset.
Regular writes are debounced and run off the UI thread; normal closure flushes
the final edit. Environment origins and thread IDs isolate drafts.

## Implemented slices, not certified parity

- Authenticated Effect RPC, bounded background delivery, JSON heartbeats,
  stream acknowledgements, request correlation and subscription cancellation.
- Live project/thread sidebar, thread detail subscriptions, manual reconnect,
  archive retrieval and historical thread search.
- Initial add-project/new-thread/rename/archive/unarchive/settle/snooze/pin/delete
  controls. Deletion requires a second confirmation click.
- Native multiline composer, send with receipt-aware clearing, interrupt/stop,
  Markdown messages and activity/session output. Provider/model IDs are manually
  editable; provider setup and catalog UI remain unimplemented.
- Draft recovery and prompt stash. Sending failure and edits made while a send
  is pending preserve the user's text.

Important gaps include pairing/Keychain, automatic reconnect, relay/DPoP,
multi-environment navigation, queue management, attachments/context, approvals,
full Markdown equivalence, plan execution, terminal rendering, source-control
UI, settings/palette/keybinding parity and embedded browser functionality.
Unimplemented thread events fail closed rather than silently showing stale state.
The existing server remains authoritative for provider execution, orchestration,
durable history and checkpoints. This port does not embed Electron.

GPUI is not a browser engine. Profiles, previews, recordings, automation and
picking require an explicit browser adapter; a URL viewer is not an equivalent.

## Verify the implementation

```sh
cargo test --manifest-path apps/native-macos/Cargo.toml
cargo clippy --manifest-path apps/native-macos/Cargo.toml --all-targets -- -D warnings
python3 -m unittest discover -s apps/native-macos/parity -p 'test_*.py'
python3 apps/native-macos/parity/differential.py --output /external/evidence/directory
python3 apps/native-macos/parity/live_smoke.py --output /external/live-evidence --confirm-isolated-server
```

The differential runner executes the **current client's production reducer**
and the Rust production reducer against common contract-normalized fixtures. It
retains both observations, comparing every state frame. These are reducer cases,
not end-to-end product scenario passes.

Headless GPUI tests exercise actual input/click dispatch and asynchronous receipt
handling. They are not a substitute for native macOS integration verification.
Automating the visible native app requires macOS Accessibility and Screen
Recording permissions for the execution host; the port does not grant these.

`parity-session` is a real-server transport/read-model runner:

```sh
cargo run --manifest-path apps/native-macos/Cargo.toml --bin parity-session < actions.json
```

Actions are `dispatch` with a client `command`, `call` with `tag`/`payload`, or
`select` with `thread_id`. Dispatch observations wait on shell sequence receipts;
selection waits on snapshot/synchronization markers. It does not automate the
GPUI window, so its results cannot certify whole UI scenarios. Use only sandbox
servers and owned test projects: dispatch actions can mutate their environment.
`live_smoke.py` builds this runner, creates an owned fixture workspace/project/thread,
checks the lifecycle observations, and retains the state and external artifacts
for inspection. It requires the same private connection environment variables.

## Evaluate product parity

```sh
python3 apps/native-macos/parity/evaluate.py evaluate
python3 apps/native-macos/parity/evaluate.py evaluate --evidence /path/to/evidence.json
```

`parity/baseline.json` pins the fork revision, source/RPC discovery inventory and
59 initial behavioral scenarios. They are workstream-level discovery scenarios,
not yet an exhaustive specification. Review every inventory unit and map it to
scenarios in `coverage_review`, or explain why it is non-behavioral infrastructure.

A scenario needs explicit `evaluation_entry_points` and `evaluation_cases` before
it can pass. Each case specifies `id`, `mode`, `entry_point`, `actions`, and an
`expected` observation object. Cases must cover all required modes and entry
points. Specify fixtures, reverse actions, persistence, errors and meaningful
performance checks where relevant; do not narrow the original feature to obtain
a passing number.

Evidence is keyed by scenario ID, then `current`/`native`. Each run supplies a
passing result, source `revision`, `artifact` path and artifact `sha256`. Native
runs additionally supply `source_sha256` matching `native_source_sha256` in the
report. Reference revisions must match the pinned fork revision.

The checksummed JSON artifact must identify `scope: "end-to-end"`, `scenario_id`
and `client`, and contain every specified case with matching mode, entry point,
actions, passing result and `observed` output equal to the specification. Arbitrary
files, reducer/transport reports, incomplete cases and a self-reported “pass” do
not qualify. Retain original UI/command/state evidence alongside these records.
Runners and independent review must still establish that the artifacts are true.

Evaluation exits nonzero until all scenarios pass, inventory review is complete,
and the reference has no drift. `verified_scenario_percent` measures only the
specified scenarios—not the percentage of the entire product implemented.

Tracking: [evaluation #44](https://github.com/ricotrevisan/t3code/issues/44).
Workstream URLs are in `parity/issues.json`.
