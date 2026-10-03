#!/usr/bin/env python3
"""Verify native RPC/read models against an isolated real fork server, NOT whole GUI scenarios."""
import argparse
import json
import os
import subprocess
import tempfile
import uuid
from datetime import datetime, timedelta, timezone
from pathlib import Path

from evaluate import native_source_digest

ROOT = Path(__file__).resolve().parents[3]
APP = ROOT / "apps/native-macos"


def command(kind, **fields):
    return {"type": kind, "commandId": str(uuid.uuid4()), **fields}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--confirm-isolated-server", action="store_true", help="Required: the selected server must use sandbox state")
    args = parser.parse_args()
    if not args.confirm_isolated_server:
        parser.error("Use a sandbox server, then explicitly confirm it; never target live T3 state")
    if not os.environ.get("T3_NATIVE_SERVER") or not os.environ.get("T3_NATIVE_ACCESS_TOKEN"):
        parser.error("Set T3_NATIVE_SERVER and T3_NATIVE_ACCESS_TOKEN privately")
    args.output.mkdir(parents=True, exist_ok=True)
    workspace = Path(tempfile.mkdtemp(prefix="native-workspace-", dir=args.output)).resolve()
    subprocess.run(["git", "init", "-q", str(workspace)], check=True)
    (workspace / "README.md").write_text("# Native parity fixture\nNo live user data.\n")
    project, thread = str(uuid.uuid4()), str(uuid.uuid4())
    now = datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")
    tomorrow = (datetime.now(timezone.utc) + timedelta(days=1)).isoformat(timespec="milliseconds").replace("+00:00", "Z")
    actions = [
        {"op": "dispatch", "command": command("project.create", projectId=project, title="Native parity sandbox", workspaceRoot=str(workspace), createdAt=now)},
        {"op": "dispatch", "command": command("thread.create", threadId=thread, projectId=project, title="Native fixture thread", modelSelection={"instanceId": "codex", "model": "gpt-5.4"}, runtimeMode="full-access", interactionMode="default", branch=None, worktreePath=None, createdAt=now)},
        {"op": "select", "thread_id": thread},
    ]
    for kind, fields in [
        ("thread.meta.update", {"title": "Native renamed fixture"}),
        ("thread.archive", {}), ("thread.unarchive", {}),
        ("thread.snooze", {"snoozedUntil": tomorrow}), ("thread.unsnooze", {"reason": "user"}),
        ("thread.pin", {}), ("thread.unpin", {}),
        ("thread.settle", {}), ("thread.unsettle", {"reason": "user"}),
        ("thread.runtime-mode.set", {"runtimeMode": "approval-required", "createdAt": now}),
        ("thread.interaction-mode.set", {"interactionMode": "plan", "createdAt": now}),
    ]:
        actions.append({"op": "dispatch", "command": command(kind, threadId=thread, **fields)})
        if kind == "thread.archive":
            actions.append({"op": "call", "tag": "orchestration.getArchivedShellSnapshot", "payload": {}})
    actions.extend([
        {"op": "select", "thread_id": thread},
        {"op": "call", "tag": "orchestration.searchThreads", "payload": {"query": "Native renamed", "limit": 50}},
    ])
    subprocess.run(["cargo", "build", "--manifest-path", str(APP / "Cargo.toml"), "--bin", "parity-session"], check=True, cwd=ROOT)
    result = subprocess.run([str(APP / "target/debug/parity-session")], input=json.dumps(actions), text=True, capture_output=True, cwd=ROOT)
    (args.output / "actions.json").write_text(json.dumps(actions, indent=2) + "\n")
    if result.returncode:
        (args.output / "failure.txt").write_text(result.stderr)
        raise SystemExit("Native runner failed; see failure.txt in the external evidence directory")
    frames = json.loads(result.stdout)["frames"]
    if len(frames) != len(actions):
        raise RuntimeError("Incomplete action transcript")
    observed = []
    for index, (action, frame) in enumerate(zip(actions, frames)):
        kind = action.get("command", {}).get("type", action["op"])
        checks = []
        if action["op"] == "dispatch":
            state = frame["state"]
            selected = next((item for item in state["threads"] if item["id"] == thread), None)
            if kind == "project.create":
                checks.append(any(item["id"] == project for item in state["projects"]))
            elif kind == "thread.archive":
                checks.append(selected is None)
            else:
                checks.append(selected is not None)
                if selected:
                    expected = {
                        "thread.meta.update": ("title", "Native renamed fixture"),
                        "thread.unsnooze": ("snoozedUntil", None),
                        "thread.unpin": ("pinnedAt", None),
                        "thread.settle": ("settledOverride", "settled"),
                        "thread.unsettle": ("settledOverride", "active"),
                        "thread.runtime-mode.set": ("runtimeMode", "approval-required"),
                        "thread.interaction-mode.set": ("interactionMode", "plan"),
                    }.get(kind)
                    if expected:
                        checks.append(selected[expected[0]] == expected[1])
                    elif kind == "thread.snooze":
                        checks.append(selected["snoozedUntil"] == tomorrow)
                    elif kind == "thread.pin":
                        checks.append(selected["pinnedAt"] is not None)
                    elif kind == "thread.unarchive":
                        checks.append(selected["archivedAt"] is None)
        elif action["op"] == "select":
            detail = frame["state"]["detail"]
            checks.append(detail["id"] == thread)
            if index > 2:
                checks.extend([detail["title"] == "Native renamed fixture", detail["runtimeMode"] == "approval-required", detail["interactionMode"] == "plan"])
        elif action["tag"] == "orchestration.getArchivedShellSnapshot":
            checks.append(any(item["id"] == thread and item["archivedAt"] is not None for item in frame["result"]["threads"]))
        else:
            checks.append(isinstance(frame["result"].get("matches"), list))
        observed.append({"step": index, "operation": kind, "result": "pass" if checks and all(checks) else "fail"})
    report = {"scope": "native transport/read-model only; not paired GUI parity", "revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(), "native_source_sha256": native_source_digest(), "project_id": project, "thread_id": thread, "workspace": str(workspace), "observations": observed, "frames": frames}
    (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    passed = sum(item["result"] == "pass" for item in observed)
    print(f"Real-server transport/read-model steps: {passed}/{len(observed)}. This does not increment the 59-scenario score.")
    raise SystemExit(0 if passed == len(observed) else 1)


if __name__ == "__main__":
    main()
