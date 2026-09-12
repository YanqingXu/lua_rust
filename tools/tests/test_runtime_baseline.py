import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile


TOOLS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOLS))
SPEC = importlib.util.spec_from_file_location("runtime_baseline", TOOLS / "run_runtime_baseline.py")
baseline = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(baseline)


class RuntimeBaselineTests(unittest.TestCase):
    def test_source_archive_freezes_uncommitted_bytes_and_rejects_concurrent_edits(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "input"
            source.mkdir()
            (source / "Cargo.toml").write_bytes(b"original\r\n")
            with patch.object(baseline, "source_paths", return_value=["Cargo.toml"]), patch.object(baseline, "git_output", return_value=b"commit"):
                record = baseline.snapshot_source(source, root / "snapshot", root / "source.zip")
                (source / "Cargo.toml").write_bytes(b"changed\n")
                self.assertEqual((root / "snapshot/Cargo.toml").read_bytes(), b"original\r\n")
                with zipfile.ZipFile(root / "source.zip") as bundle:
                    self.assertEqual(bundle.read("Cargo.toml"), b"original\r\n")
                self.assertEqual(record["files_sha256"]["Cargo.toml"], baseline.digest(b"original\r\n"))
            with patch.object(baseline, "source_paths", side_effect=[["Cargo.toml"], []]):
                with self.assertRaisesRegex(ValueError, "changed during snapshot"):
                    baseline.snapshot_source(source, root / "mixed", root / "mixed.zip")

    def test_success_requires_each_workload_and_complete_sample_counts(self):
        parameters = {"iterations": 1, "width": 1, "batches": 1, "warmup": 0}
        summary = {"samples": 1, "min_ns": 10, "max_ns": 10, "p50_ns": 10, "p95_ns": 10, "p99_ns": 10}
        record = {"schema_version": 1, "completed": True, "parameters": parameters, "workloads": [
            {"name": "table-reinsert", "runtime_instances": 1, "batches": [{}], "execution_wall": summary, "explicit_full_gc_wall": summary}
        ]}
        baseline.validate_measurement(record, parameters, "table-reinsert")
        with self.assertRaisesRegex(ValueError, "every selected workload"):
            baseline.validate_measurement(record, parameters, "all")
        copy = json.loads(json.dumps(record))
        copy["workloads"][0]["execution_wall"]["p99_ns"] = 9
        with self.assertRaisesRegex(ValueError, "latency"):
            baseline.validate_measurement(copy, parameters, "table-reinsert")
        record["completed"] = False
        with self.assertRaisesRegex(ValueError, "did not complete"):
            baseline.validate_measurement(record, parameters, "table-reinsert")


if __name__ == "__main__":
    unittest.main()
