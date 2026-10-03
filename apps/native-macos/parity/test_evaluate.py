import copy
import hashlib
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import evaluate


class ParityEvaluationTests(unittest.TestCase):
    def setUp(self):
        self.baseline = {
            "revision": "a" * 40,
            "items": [{"id": "source", "sha256": "abc"}],
            "coverage_review": {"source": {"scenarios": ["send"]}},
            "scenarios": [{"id": "send", "group": "conversation"}],
        }
        self.patch = patch.object(evaluate, "inventory", return_value=self.baseline["items"])
        self.patch.start()
        self.addCleanup(self.patch.stop)
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.artifact = Path(self.directory.name) / "result.json"
        self.artifact.write_text('{"visible": "reply", "durable": "reply"}')
        self.evidence = {"send": {client: {
            "result": "pass", "revision": self.baseline["revision"],
            "artifact": str(self.artifact),
            "sha256": hashlib.sha256(self.artifact.read_bytes()).hexdigest(),
            "source_sha256": evaluate.native_source_digest(),
        } for client in ("current", "native")}}

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


if __name__ == "__main__":
    unittest.main()
