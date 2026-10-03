#!/usr/bin/env python3
"""Compare production thread reducers. These cases DO NOT certify whole UI scenarios."""
import argparse
import copy
import json
import subprocess
from pathlib import Path

from evaluate import native_source_digest

ROOT = Path(__file__).resolve().parents[3]
APP = ROOT / "apps/native-macos"
DATE = "2026-04-01T00:00:00.000Z"
LATER = "2026-04-01T01:00:00.000Z"
BASE = {
    "id": "thread-1", "projectId": "project-1", "title": "Test Thread",
    "modelSelection": {"instanceId": "codex", "model": "gpt-5.4"},
    "runtimeMode": "full-access", "interactionMode": "default", "branch": None,
    "worktreePath": None, "latestTurn": None, "createdAt": DATE, "updatedAt": DATE,
    "archivedAt": None, "settledOverride": None, "settledAt": None,
    "pullRequests": [], "deletedAt": None, "messages": [], "proposedPlans": [],
    "activities": [], "checkpoints": [], "session": None,
}


def event(kind, **payload):
    return {"sequence": 1, "eventId": "event-1", "aggregateKind": "thread", "aggregateId": "thread-1",
            "occurredAt": LATER, "commandId": None, "causationEventId": None,
            "correlationId": None, "metadata": {}, "type": kind,
            "payload": {"threadId": "thread-1", **payload}}


def session(status="running", **fields):
    return {"threadId": "thread-1", "status": status, "providerName": "codex", "runtimeMode": "full-access",
            "activeTurnId": "turn-1" if status == "running" else None, "lastError": None, "updatedAt": LATER, **fields}


def message(text, streaming=True, **fields):
    return event("thread.message-sent", messageId="message-1", role="assistant", text=text,
                 turnId="turn-1", streaming=streaming, createdAt=DATE, updatedAt=LATER, **fields)


def cases():
    result = []
    def add(name, parent, events, **state):
        thread = copy.deepcopy(BASE)
        thread.update(state)
        for i, entry in enumerate(events, 1):
            entry["sequence"] = i
            entry["eventId"] = f"event-{i}"
        result.append({"id": name, "parent_scenario": parent, "input": {"thread": thread, "events": events}})
    add("archive-unarchive", "threads-02", [event("thread.archived", archivedAt=LATER, updatedAt=LATER), event("thread.unarchived", updatedAt=LATER)])
    add("settle-unsettle-user", "threads-01", [event("thread.settled", settledAt=LATER, updatedAt=LATER), event("thread.unsettled", reason="user", updatedAt=LATER)])
    add("activity-unsettle", "threads-01", [event("thread.unsettled", reason="activity", updatedAt=LATER)], settledOverride="settled", settledAt=DATE)
    add("active-reentry-stamp", "threads-01", [event("thread.unsettled", reason="activity", updatedAt=LATER)], settledOverride="active", unsettledAt=DATE)
    add("snooze-unsnooze", "threads-03", [event("thread.snoozed", snoozedUntil="2026-04-02T00:00:00.000Z", snoozedAt=LATER, updatedAt=LATER), event("thread.unsnoozed", reason="user", updatedAt=LATER)])
    add("pin-unpin", "threads-01", [event("thread.pinned", pinnedAt=LATER, pinOrderKey="a0", updatedAt=LATER), event("thread.unpinned", updatedAt=LATER)])
    add("pin-reorder", "threads-01", [event("thread.pin-reordered", orderKey="a1", updatedAt=LATER)], pinnedAt=DATE, pinOrderKey="a0")
    add("auto-settle-disable-enable", "threads-01", [event("thread.auto-settle-set", autoSettleDisabledAt=LATER, updatedAt=LATER), event("thread.auto-settle-set", autoSettleDisabledAt=None, updatedAt=LATER)])
    add("rename-and-branch-clear", "threads-01", [event("thread.meta-updated", title="Renamed", branch=None, updatedAt=LATER)], branch="feature", worktreePath="/tmp/worktree")
    add("title-refinement-state", "threads-01", [event("thread.meta-updated", titleState={"source":"manual","version":"command-1","needsRefinement":False}, titleRegeneration=None, updatedAt=LATER)])
    add("runtime-mode-toggle", "providers-01", [event("thread.runtime-mode-set", runtimeMode="approval-required", updatedAt=LATER), event("thread.runtime-mode-set", runtimeMode="full-access", updatedAt=LATER)])
    add("plan-mode-toggle", "conversation-06", [event("thread.interaction-mode-set", interactionMode="plan", updatedAt=LATER), event("thread.interaction-mode-set", interactionMode="default", updatedAt=LATER)])
    add("turn-start-selection", "conversation-02", [event("thread.turn-start-requested", messageId="user-1", modelSelection={"instanceId":"claudeAgent","model":"sonnet"}, runtimeMode="full-access", interactionMode="default", createdAt=LATER)])
    add("streaming-append-final-replace", "conversation-02", [message("Hello"), message(", 世界 🌍"), message("Hello, 世界 🌍", False)])
    add("empty-completion-retains-text", "conversation-02", [message("Hello"), message("", False)])
    add("reasoning-stays-separate", "conversation-02", [event("thread.message-sent", messageId="reasoning-1", role="reasoning", text="Thinking", turnId="turn-1", streaming=True, createdAt=DATE, updatedAt=LATER), message("Answer", False)])
    add("context-and-attachments-survive-deltas", "conversation-01", [event("thread.message-sent", messageId="user-1", role="user", text="Question", turnId=None, streaming=False, createdAt=DATE, updatedAt=LATER, attachments=[]), message("Answer", False)])
    add("assistant-completion-does-not-settle-running-session", "conversation-02", [message("Commentary", False)], session=session())
    add("ready-session-settles-whole-turn", "conversation-02", [event("thread.session-set", session=session()), message("Commentary", False), event("thread.session-set", session=session("ready"))])
    add("error-session-settles-as-error", "conversation-02", [event("thread.session-set", session=session()), event("thread.session-set", session=session("error", lastError="Fixture failure"))])
    add("stop-session-with-no-session", "conversation-03", [event("thread.session-stop-requested", createdAt=LATER)])
    add("stop-session-clears-active-turn", "conversation-03", [event("thread.session-stop-requested", createdAt=LATER)], session=session())
    add("interrupt-current-turn", "conversation-03", [event("thread.session-set", session=session()), event("thread.turn-interrupt-requested", turnId="turn-1", createdAt=LATER)])
    add("interrupt-other-turn-is-noop", "conversation-03", [event("thread.session-set", session=session()), event("thread.turn-interrupt-requested", turnId="turn-other", createdAt=LATER)])
    add("interrupt-without-turn-is-noop", "conversation-03", [event("thread.turn-interrupt-requested", createdAt=LATER)])
    plan = {"id":"plan-1","turnId":"turn-1","planMarkdown":"# Plan\n\nDo work","implementedAt":None,"implementationThreadId":None,"createdAt":DATE,"updatedAt":LATER}
    add("plan-replacement-and-order", "conversation-06", [event("thread.proposed-plan-upserted", proposedPlan=plan), event("thread.proposed-plan-upserted", proposedPlan={**plan,"planMarkdown":"# Revised"})])
    activity = {"id":"activity-1","tone":"tool","kind":"tool.completed","summary":"Ran command","payload":{"exitCode":0,"output":"ok"},"turnId":"turn-1","sequence":2,"createdAt":DATE}
    add("activity-deduplication", "conversation-02", [event("thread.activity-appended", activity=activity), event("thread.activity-appended", activity=activity)])
    add("activity-order", "conversation-02", [event("thread.activity-appended", activity=activity), event("thread.activity-appended", activity={**activity,"id":"activity-2","sequence":1})])
    context = {**activity,"kind":"context-window.updated","payload":{"usedTokens":10}}
    add("context-window-coalescing", "conversation-02", [event("thread.activity-appended", activity=context), event("thread.activity-appended", activity={**context,"id":"activity-2","payload":{"usedTokens":20}})])
    add("invalid-context-does-not-remove-resolvable-context", "conversation-02", [event("thread.activity-appended", activity=context), event("thread.activity-appended", activity={**context,"id":"activity-2","payload":{"usedTokens":-1}})])
    checkpoint = {"turnId":"turn-1","checkpointTurnCount":1,"checkpointRef":"refs/t3/checkpoint/1","status":"ready","files":[],"assistantMessageId":None,"completedAt":LATER}
    add("checkpoint-ready-completes-turn", "source-control-03", [event("thread.turn-diff-completed", **checkpoint)])
    add("mid-turn-checkpoint-does-not-settle", "source-control-03", [event("thread.session-set", session=session()), event("thread.turn-diff-completed", **checkpoint)])
    add("missing-checkpoint-cannot-downgrade-ready", "source-control-03", [event("thread.turn-diff-completed", **checkpoint), event("thread.turn-diff-completed", **{**checkpoint,"status":"missing"})])
    add("assistant-rebinds-checkpoint", "source-control-03", [event("thread.turn-diff-completed", **checkpoint), message("Done", False)])
    add("conversation-revert-clears-turn", "source-control-03", [message("Done", False), event("thread.turn-diff-completed", **checkpoint), event("thread.reverted", turnCount=0)])
    imported = {"id":"import:session:message","role":"assistant","text":"Imported history","turnId":None,"streaming":False,"createdAt":DATE,"updatedAt":DATE}
    add("revert-preserves-imported-history", "threads-05", [event("thread.reverted", turnCount=0)], messages=[imported])
    fallback_messages = [
        {**imported,"id":"user-utc","role":"user","text":"Later absolute time","createdAt":"2026-04-01T00:30:00.000Z"},
        {**imported,"id":"user-offset","role":"user","text":"Earlier absolute time","createdAt":"2026-04-01T01:00:00.000+02:00"},
    ]
    add("revert-fallback-compares-absolute-time", "source-control-03", [event("thread.reverted", turnCount=1)], messages=fallback_messages)
    add("thread-delete", "threads-01", [event("thread.deleted", deletedAt=LATER)])
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True, help="External evidence directory; never commit run artifacts")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    subprocess.run(["cargo", "build", "--manifest-path", str(APP / "Cargo.toml"), "--bin", "parity-replay"], cwd=ROOT, check=True)
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    reference_revision = json.loads((APP / "parity/baseline.json").read_text())["revision"]
    source_digest = native_source_digest()
    results = []
    for case in cases():
        reference = subprocess.run(["node", str(APP / "parity/reference-replay.ts")], input=json.dumps(case["input"]), text=True, capture_output=True, cwd=ROOT)
        if reference.returncode:
            results.append({"id":case["id"],"parent_scenario":case["parent_scenario"],"result":"invalid-fixture","error":reference.stderr})
            continue
        current = json.loads(reference.stdout)
        native = subprocess.run([str(APP / "target/debug/parity-replay")], input=json.dumps(current["normalized_input"]), text=True, capture_output=True, cwd=ROOT)
        observed = json.loads(native.stdout) if native.returncode == 0 else {"error":native.stderr}
        passed = native.returncode == 0 and observed["frames"] == current["frames"]
        artifact = {"revision":revision,"reference_revision":reference_revision,"native_source_sha256":source_digest,"case":case,"current":current,"native":observed,"result":"pass" if passed else "fail","scope":"production reducer only; not end-to-end UI parity"}
        (args.output / f"{case['id']}.json").write_text(json.dumps(artifact, indent=2)+"\n")
        results.append({"id":case["id"],"parent_scenario":case["parent_scenario"],"result":artifact["result"]})
    report = {"revision":revision,"reference_revision":reference_revision,"native_source_sha256":source_digest,"scope":"production reducer differential cases; these do not certify the 59 product scenarios", "passed":sum(r["result"] == "pass" for r in results), "total":len(results), "results":results}
    (args.output / "report.json").write_text(json.dumps(report,indent=2)+"\n")
    print(json.dumps(report,indent=2))
    raise SystemExit(0 if report["passed"] == report["total"] else 1)


if __name__ == "__main__":
    main()
