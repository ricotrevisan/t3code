#!/usr/bin/env python3
"""Inventory the fork and fail closed until paired behavioral evidence exists."""
import argparse
import hashlib
import json
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent

GROUPS = {
    "connections": ("Connections, authentication and environments", ["pair", "connect", "machine", "environment", "auth", "relay", "device", "remote", "background"]),
    "projects": ("Projects, worktrees and filesystem", ["project", "filesystem", "worktree", "files", "assets", "attachment"]),
    "threads": ("Thread lifecycle, navigation and search", ["thread", "sidebar", "search", "orchestration", "agentSession"]),
    "conversation": ("Composer, streaming, approvals and rendering", ["chat", "composer", "markdown", "question", "plan", "queue", "context"]),
    "providers": ("All six providers, models, setup and usage", ["provider", "model", "usage", "codex", "claude", "cursor", "grok", "opencode", "antigravity"]),
    "source-control": ("Git, checkpoints, diffs and pull requests", ["vcs", "git", "diff", "review", "pullRequest", "source-control"]),
    "terminal": ("PTY terminals and project scripts", ["terminal", "script"]),
    "preview": ("Browser, preview, automation and capture", ["preview", "browser", "capture", "snap", "annotation"]),
    "settings": ("Scoped settings, appearance and shortcuts", ["settings", "theme", "appearance", "keybinding", "palette", "font"]),
    "macos": ("Native shell, permissions, updates and distribution", []),
}


def group(name):
    for key, (_, words) in GROUPS.items():
        if any(word.lower() in name.lower() for word in words):
            return key
    return "macos"


def inventory():
    items = []
    # Discovery units are coverage sentinels, not claims that a file is one feature.
    for directory in ["apps/web/src", "apps/desktop/src", "packages/client-runtime/src", "packages/contracts/src"]:
        for path in sorted((ROOT / directory).rglob("*")):
            if path.suffix not in (".ts", ".tsx", ".css"):
                continue
            relative = path.relative_to(ROOT).as_posix()
            items.append({"id": relative, "group": group(relative), "sha256": hashlib.sha256(path.read_bytes()).hexdigest(), "kind": "source"})
    for path in sorted((ROOT / "packages/contracts/src").glob("*.ts")):
        if ".test." in path.name:
            continue
        text = path.read_text()
        methods = set(re.findall(r'Rpc\.make\(\s*["\']([^"\']+)', text))
        for block in re.findall(r'export const \w*WS_METHODS\s*=\s*\{(.*?)\}', text, re.S):
            methods.update(re.findall(r':\s*["\']([^"\']+)', block))
        for method in sorted(methods):
            items.append({"id": f"rpc:{method}", "group": group(method), "source": path.relative_to(ROOT).as_posix(), "kind": "rpc"})
    return items


def native_source_digest():
    digest = hashlib.sha256()
    app = HERE.parent
    paths = list((app / "src").rglob("*.rs")) + [app / "Cargo.toml", app / "Cargo.lock"]
    for path in sorted(paths):
        if path.is_file():
            digest.update(path.relative_to(app).as_posix().encode())
            digest.update(path.read_bytes())
    return digest.hexdigest()


def behavioral_artifact(path, scenario, client):
    try:
        document = json.loads(path.read_text())
    except (OSError, ValueError):
        return False
    cases = scenario.get("evaluation_cases", [])
    entry_points = scenario.get("evaluation_entry_points", [])
    if not cases or not entry_points:
        return False
    if set(scenario.get("required_modes", [])) - {case.get("mode") for case in cases}:
        return False
    if set(entry_points) - {case.get("entry_point") for case in cases}:
        return False
    if document.get("scope") != "end-to-end" or document.get("scenario_id") != scenario["id"] or document.get("client") != client:
        return False
    observed = document.get("cases", [])
    case_ids = [case.get("id") for case in cases]
    if len(set(case_ids)) != len(case_ids) or not all(case_ids):
        return False
    if len(observed) != len(cases) or {case.get("id") for case in observed} != set(case_ids):
        return False
    by_id = {case["id"]: case for case in observed}
    for expected in cases:
        actual = by_id[expected["id"]]
        if not expected.get("actions") or not expected.get("expected"):
            return False
        if actual.get("result") != "pass":
            return False
        if any(actual.get(key) != expected.get(key) for key in ("mode", "entry_point", "actions")):
            return False
        if actual.get("observed") != expected["expected"]:
            return False
    return True


def evaluate(baseline, evidence):
    current = inventory()
    changed = {x["id"] for x in current} ^ {x["id"] for x in baseline["items"]}
    old = {x["id"]: x for x in baseline["items"]}
    changed.update(x["id"] for x in current if x["id"] in old and x.get("sha256") != old[x["id"]].get("sha256"))
    scenario_ids = {s["id"] for s in baseline["scenarios"]}
    if len(scenario_ids) != len(baseline["scenarios"]):
        raise ValueError("Duplicate scenario IDs invalidate the evaluation")
    coverage = baseline.get("coverage_review", {})
    unreviewed = []
    for item in baseline["items"]:
        review = coverage.get(item["id"], {})
        mapped = review.get("scenarios", [])
        # Infrastructure-only files still need an explicit, reviewable rationale.
        if not (mapped and set(mapped) <= scenario_ids) and not review.get("non_behavioral_reason"):
            unreviewed.append(item["id"])
    rows = []
    for scenario in baseline["scenarios"]:
        proof = evidence.get(scenario["id"], {})
        paired = []
        for client in ("current", "native"):
            run = proof.get(client, {})
            artifact = run.get("artifact", "")
            artifact_path = ROOT / artifact
            valid = bool(artifact and artifact_path.is_file() and run.get("sha256") == hashlib.sha256(artifact_path.read_bytes()).hexdigest())
            revision_valid = run.get("revision") == baseline["revision"] if client == "current" else bool(re.fullmatch(r"[0-9a-f]{40}", run.get("revision", "")))
            source_valid = client == "current" or run.get("source_sha256") == native_source_digest()
            behavioral_valid = valid and behavioral_artifact(artifact_path, scenario, client)
            paired.append(run.get("result") == "pass" and revision_valid and source_valid and behavioral_valid)
        rows.append({"id": scenario["id"], "group": scenario["group"], "status": "pass" if all(paired) else "unverified"})
    passed = sum(row["status"] == "pass" for row in rows)
    return {"baseline": baseline["revision"], "native_source_sha256": native_source_digest(), "coverage_drift": sorted(changed), "unreviewed_inventory": unreviewed, "passed": passed, "total": len(rows), "verified_scenario_percent": round(100 * passed / len(rows), 2) if rows else 0, "complete": bool(rows) and passed == len(rows) and not changed and not unreviewed, "scenarios": rows}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["inventory", "evaluate"])
    parser.add_argument("--evidence", type=Path)
    args = parser.parse_args()
    if args.command == "inventory":
        print(json.dumps({"revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(), "items": inventory()}, indent=2))
        return
    baseline = json.loads((HERE / "baseline.json").read_text())
    evidence = json.loads(args.evidence.read_text()) if args.evidence else {}
    report = evaluate(baseline, evidence)
    print(json.dumps(report, indent=2))
    raise SystemExit(0 if report["complete"] else 1)


if __name__ == "__main__":
    main()
