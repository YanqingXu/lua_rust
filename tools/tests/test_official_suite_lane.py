import importlib.util
import contextlib
import io
import json
from pathlib import Path
import sys
import tempfile
import time
import unittest


MODULE = Path(__file__).resolve().parents[1] / "run_official_suite.py"
SPEC = importlib.util.spec_from_file_location("official_suite", MODULE)
lane = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(lane)


class OfficialSuiteLaneTests(unittest.TestCase):
    def test_required_case_cannot_be_converted_to_a_skipped_role(self):
        root = MODULE.parents[1]
        original = root / "tests/compatibility/official-source-lane.json"
        with tempfile.TemporaryDirectory() as directory:
            manifest = json.loads(original.read_text(encoding="utf-8"))
            path = Path(directory) / "manifest.json"
            for role in ("excluded", "helper"):
                with self.subTest(role=role):
                    case = next(case for case in manifest["cases"] if case["file"] == "constructs.lua")
                    case["role"] = role
                    path.write_text(json.dumps(manifest), encoding="utf-8")
                    error = io.StringIO()
                    with contextlib.redirect_stderr(error), self.assertRaises(SystemExit) as result:
                        lane.main([
                            "--manifest", str(path), "--case", "constructs.lua",
                            "--rust", sys.executable, "--official", sys.executable,
                            "--artifact", str(Path(directory) / "result.json"),
                        ])
                    self.assertEqual(result.exception.code, 2)
                    self.assertIn("explicitly selected cases must run", error.getvalue())

    def test_binary_snapshot_is_immutable_when_the_input_is_rebuilt(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "lua.exe"
            binary.write_bytes(b"original executable")
            (root / "runtime.dll").write_bytes(b"original dependency")
            snapshots, identities = lane.snapshot_engines({"lua_rust": binary}, root / "snapshot")
            binary.write_bytes(b"replacement executable")
            self.assertEqual(snapshots["lua_rust"].read_bytes(), b"original executable")
            self.assertEqual(identities["lua_rust"]["sha256"], lane.digest(b"original executable"))
            self.assertEqual(identities["lua_rust"]["adjacent_dll_sha256"]["runtime.dll"], lane.digest(b"original dependency"))

    def test_exit_zero_without_completion_is_not_a_pass(self):
        observation = {"timed_out": False, "exit_code": 0, "stdout": {"text": "OK\n"}}
        self.assertEqual(lane.classify(observation, "calls.lua"), "missing-completion")
        observation["stdout"]["text"] += "__OFFICIAL_SOURCE_COMPLETED__:calls.lua\n"
        self.assertEqual(lane.classify(observation, "calls.lua"), "passed")
        observation["exit_code"] = 1
        self.assertEqual(lane.classify(observation, "calls.lua"), "failed")
        observation["exit_code"] = -1073740791
        self.assertEqual(lane.classify(observation, "calls.lua"), "abnormal-exit")
        observation["timed_out"] = True
        self.assertEqual(lane.classify(observation, "calls.lua"), "timeout")

    def test_timeout_terminates_descendants_and_preserves_output(self):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            child = "import time; time.sleep(1); open('escaped', 'w').write('alive')"
            parent = (
                "import subprocess, sys, time; "
                f"subprocess.Popen([sys.executable, '-c', {child!r}]); "
                "print('started', flush=True); time.sleep(30)"
            )
            observed = lane.observe_process([sys.executable, "-c", parent], work, 0.2)
            self.assertTrue(observed["timed_out"])
            self.assertIn("started", observed["stdout"]["text"])
            time.sleep(1.1)
            self.assertFalse((work / "escaped").exists())

    def test_inventory_requires_every_lua_source_and_rejects_source_edits(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            source.mkdir()
            (source / "one.lua").write_bytes(b"return 1\r\n")
            (root / "provenance.json").write_text(json.dumps({"entries": [
                {"path": "one.lua", "sha256": lane.digest(b"return 1\n")},
                {"path": "etc/ltests.c", "sha256": "unavailable"},
            ]}))
            path = root / "manifest.json"
            manifest = {"schema_version": 1, "source_root": "source", "source_provenance": "provenance.json", "cases": [{"file": "one.lua", "role": "run", "reason": "test"}]}
            path.write_text(json.dumps(manifest))
            loaded, _, hashes = lane.load_inventory(root, path)
            self.assertEqual(loaded["unavailable_provenance_assets"], ["etc/ltests.c"])
            self.assertEqual(hashes["one.lua"]["working_tree_sha256"], lane.digest(b"return 1\r\n"))
            (source / "one.lua").write_bytes(b"return 2\n")
            with self.assertRaisesRegex(ValueError, "provenance mismatch"):
                lane.load_inventory(root, path)
            manifest["cases"] = []
            path.write_text(json.dumps(manifest))
            with self.assertRaisesRegex(ValueError, "inventory"):
                lane.load_inventory(root, path)


if __name__ == "__main__":
    unittest.main()
