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

Enter the HTTP(S) server origin and a pairing URL or one-time token, then pair.
Pairing URLs must belong to that origin. Pairing exchanges the bootstrap token for
a bearer access credential; the input is masked and cleared after submission.
Use **Remember in Keychain** to retain an access credential and **Connect remembered**
to reconnect to the entered origin after relaunch. **Forget credential** disconnects
and removes that origin's local credential; it does not revoke the server session.
Keychain operations run on an ordered background worker, so forgetting cannot be
undone by an earlier delayed save. Plain HTTP is accepted only for loopback;
remote origins require HTTPS. DPoP and relay pairing are not implemented yet.

For development, `T3_NATIVE_SERVER` and `T3_NATIVE_ACCESS_TOKEN` optionally supply
an existing bearer **access token**, scoped to that server origin. Do not put
credentials in command-line arguments or use a pairing token as an access token.
Every WebSocket connection, including automatic recovery, acquires a fresh ticket.
Disconnected and pre-synchronization mutations are rejected, not buffered. A lost
receipt does not cause the mutation to be replayed.

Native drafts and stashed prompts use a separate data directory:
`~/Library/Application Support/T3 Code Native`. `T3_NATIVE_HOME` overrides this
for isolated testing. Corrupt or newer-version storage is preserved, not reset.
Regular writes are debounced and run off the UI thread; normal closure flushes
the final edit. Environment origins and thread IDs isolate drafts.

## Implemented slices, not certified parity

- Authenticated Effect RPC, bounded background delivery, JSON heartbeats,
  stream acknowledgements, request correlation and subscription cancellation.
  Automatic recovery replaces stale projections and restores thread subscription;
  authentication/protocol failures stop rather than retrying forever.
- Live project/thread sidebar, thread detail subscriptions, manual reconnect,
  archive retrieval and historical thread search.
- Initial add-project/new-thread/rename/archive/unarchive/settle/snooze/pin/delete
  controls. Deletion requires a second confirmation click. Separate active,
  explicitly settled, snoozed and archived views expose the reverse actions.
  Custom snoozes accept future ISO date/times with a timezone; one-shot timers
  refresh membership at expiry without continuous repainting. Automatic settlement
  policy and full sidebar ordering still need parity work.
- Initial project rename and removal from chat controls and the palette. Removal
  confirms before dispatch and does not force active sessions. File read/edit/save
  uses the selected environment's server RPCs, never the client's local filesystem.
  Saves target the loaded file even if the path field changes; pending responses
  preserve later edits. Truncated and external files are read-only. Server-backed
  directory browsing and bounded file-path search open entries in the editor;
  results from a previous project or connection cannot replace the active listing.
  Content search, complete picker entry points, clone and worktree UI still need
  implementation.
- Native multiline composer, send with receipt-aware clearing, interrupt/stop,
  Markdown messages and activity/session output. The server provider/model catalog
  loads after shell synchronization, exposes instance identities and availability,
  and lets users select models without typing IDs. Unchanged selections preserve
  provider options; advertised new-thread model-change restrictions are enforced.
  Manual IDs remain available. Provider installation/authentication, model options,
  and full catalog/settings entry-point parity remain unimplemented.
- Draft recovery and prompt stash. Sending failure and edits made while a send
  is pending preserve the user's text.
- Searchable native command palette (`Cmd+K`) with keyboard selection, Escape
  dismissal and focus restoration. Initial menus and fork-default shortcuts
  expose new-thread, stash, settlement and pin actions. Palette deletion uses
  the same confirmation as chat controls. Custom keybinding configuration and
  the complete current-client command catalog still require parity work.

Important gaps include full connection-management UI and relay/DPoP,
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
The Mac must also be unlocked. The driver reports a locked screen independently
of permission status and refuses input and capture while it is locked.

```sh
swift apps/native-macos/parity/native-driver.swift <owned-native-pid> inspect
swift apps/native-macos/parity/native-driver.swift <owned-native-pid> capture /external/new-window.png
swift apps/native-macos/parity/native-driver.swift <owned-native-pid> click <window-x> <window-y>
printf 'Fixture text' | swift apps/native-macos/parity/native-driver.swift <owned-native-pid> type
```

The native adapter accepts only the experimental executable, raises its owned
window and verifies it is frontmost before sending input. Click coordinates are
window-relative and bounded to that window. `key` accepts a small explicit set;
`close` uses the native close button. Observations are printed as JSON, and text
is read from stdin rather than argv. Captures are restricted to the owned window
and refuse to overwrite existing evidence. Coordinate input does **not** establish
accessibility parity: GPUI 0.2.2 currently lacks usable control semantics in the
macOS accessibility tree. This is an outstanding product defect, not a permission
problem.

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
