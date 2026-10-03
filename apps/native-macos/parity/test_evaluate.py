import copy
import hashlib
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import evaluate


class ParityEvaluationTests(unittest.TestCase):
    def setUp(self):
        case = {"id": "send-local-chat", "mode": "local", "entry_point": "chat",
                "actions": ["type prompt", "send", "wait for turn receipt"],
                "expected": {"visible": "reply", "durable": "reply", "turns": 1}}
        self.baseline = {
            "revision": "a" * 40,
            "items": [{"id": "source", "sha256": "abc"}],
            "coverage_review": {"source": {"scenarios": ["send"]}},
            "scenarios": [{"id": "send", "group": "conversation", "required_modes": ["local"],
                           "evaluation_entry_points": ["chat"], "evaluation_cases": [case]}],
        }
        self.patch = patch.object(evaluate, "inventory", return_value=self.baseline["items"])
        self.patch.start()
        self.addCleanup(self.patch.stop)
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.evidence = {"send": {}}
        self.documents = {}
        for client in ("current", "native"):
            self.documents[client] = {"scope": "end-to-end", "scenario_id": "send", "client": client,
                                      "cases": [{**case, "result": "pass", "observed": case["expected"]}]}
            self.evidence["send"][client] = {"result": "pass", "revision": self.baseline["revision"],
                                           "source_sha256": evaluate.native_source_digest()}
            self.write_artifact(client)
        self.artifact = Path(self.evidence["send"]["current"]["artifact"])

    def write_artifact(self, client):
        artifact = Path(self.directory.name) / f"{client}.json"
        artifact.write_text(json.dumps(self.documents[client]))
        self.evidence["send"][client].update(artifact=str(artifact), sha256=hashlib.sha256(artifact.read_bytes()).hexdigest())

    def test_no_evidence_never_passes(self):
        report = evaluate.evaluate(self.baseline, {})
        self.assertFalse(report["complete"])
        self.assertEqual(report["passed"], 0)

    def test_only_paired_valid_artifacts_can_pass(self):
        self.assertTrue(evaluate.evaluate(self.baseline, self.evidence)["complete"])
        del self.evidence["send"]["current"]
        self.assertFalse(evaluate.evaluate(self.baseline, self.evidence)["complete"])

    def test_missing_or_modified_artifact_invalidates_pass(self):
        self.artifact.write_text("changed")
        self.assertFalse(evaluate.evaluate(self.baseline, self.evidence)["complete"])
        self.artifact.unlink()
        self.assertFalse(evaluate.evaluate(self.baseline, self.evidence)["complete"])

    def test_stale_reference_or_native_source_invalidates_pass(self):
        self.evidence["send"]["current"]["revision"] = "b" * 40
        self.assertFalse(evaluate.evaluate(self.baseline, self.evidence)["complete"])
        self.evidence["send"]["current"]["revision"] = self.baseline["revision"]
        self.evidence["send"]["native"]["source_sha256"] = "stale"
        self.assertFalse(evaluate.evaluate(self.baseline, self.evidence)["complete"])

    def test_new_or_changed_reference_source_blocks_completion(self):
        with patch.object(evaluate, "inventory", return_value=[{"id": "source", "sha256": "new"}, {"id": "new-feature"}]):
            report = evaluate.evaluate(self.baseline, self.evidence)
        self.assertFalse(report["complete"])
        self.assertEqual(report["coverage_drift"], ["new-feature", "source"])

    def test_unaudited_inventory_blocks_completion(self):
        baseline = copy.deepcopy(self.baseline)
        baseline["coverage_review"] = {}
        report = evaluate.evaluate(baseline, self.evidence)
        self.assertFalse(report["complete"])
        self.assertEqual(report["unreviewed_inventory"], ["source"])

    def test_duplicate_scenarios_are_rejected(self):
        self.baseline["scenarios"] *= 2
        with self.assertRaises(ValueError):
            evaluate.evaluate(self.baseline, self.evidence)

    def test_reducer_or_transport_evidence_cannot_count_as_ui_parity(self):
        self.documents["native"]["scope"] = "production reducer only"
        self.write_artifact("native")
        self.assertEqual(evaluate.evaluate(self.baseline, self.evidence)["passed"], 0)

    def test_matching_checksum_does_not_hide_incorrect_observed_behavior(self):
        self.documents["native"]["cases"][0]["observed"] = {"visible": "reply", "durable": "lost", "turns": 2}
        self.write_artifact("native")
        self.assertEqual(evaluate.evaluate(self.baseline, self.evidence)["passed"], 0)

    def test_omitted_modes_entry_points_and_cases_block_pass(self):
        for key, value in [("required_modes", ["local", "relay/tunnel"]),
                           ("evaluation_entry_points", ["chat", "command-palette"]),
                           ("evaluation_cases", [])]:
            baseline = copy.deepcopy(self.baseline)
            baseline["scenarios"][0][key] = value
            self.assertEqual(evaluate.evaluate(baseline, self.evidence)["passed"], 0)
        self.documents["native"]["cases"] = []
        self.write_artifact("native")
        self.assertEqual(evaluate.evaluate(self.baseline, self.evidence)["passed"], 0)

    def test_artifact_from_wrong_client_cannot_be_reused(self):
        self.evidence["send"]["native"].update(artifact=self.evidence["send"]["current"]["artifact"], sha256=self.evidence["send"]["current"]["sha256"])
        self.assertEqual(evaluate.evaluate(self.baseline, self.evidence)["passed"], 0)


if __name__ == "__main__":
    unittest.main()
