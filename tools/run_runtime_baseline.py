"""Build a frozen source snapshot and observe repeated Runtime workload batches.

The benchmark reports host wall time and managed accounting, not automatic GC
pauses or OS RSS. This wrapper intentionally makes no cross-engine comparison.
"""

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import uuid
import zipfile

from run_official_suite import digest, observe_process, snapshot_engines


SOURCE_ROOTS = ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo", "crates"]
TOOL_FILES = ["tools/run_runtime_baseline.py", "tools/run_official_suite.py"]


def git_output(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args])


def source_paths(root):
    names = git_output(root, "ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", *SOURCE_ROOTS)
    # Deleted tracked files are not inputs to the current working tree build.
    return sorted({name.decode("utf-8") for name in names.split(b"\0") if name and (root / name.decode("utf-8")).exists()} | set(TOOL_FILES))


def snapshot_source(root, destination, archive):
    paths = source_paths(root)
    hashes = {}
    for name in paths:
        source = root / name
        if source.is_symlink() or not source.resolve().is_relative_to(root.resolve()):
            raise ValueError(f"source snapshot requires workspace-local regular files: {name}")
        data = source.read_bytes()
        target = destination / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        hashes[name] = digest(data)
    # Reject a concurrent edit instead of silently naming a mixed-time source
    # snapshot after HEAD. Uncommitted inputs are included and individually hashed.
    if paths != source_paths(root) or any(digest((root / name).read_bytes()) != sha for name, sha in hashes.items()):
        raise ValueError("source inputs changed during snapshot; retry after edits settle")
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as bundle:
        for name in paths:
            bundle.write(destination / name, name)
    manifest = json.dumps(hashes, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return {
        "input_root": str(root), "snapshot_root": str(destination),
        "archive": str(archive), "archive_sha256": digest(archive.read_bytes()),
        "manifest_sha256": digest(manifest), "files_sha256": hashes,
        "scope": "Cargo workspace/config/toolchain/crates plus benchmark runner and process helper; original file bytes including uncommitted inputs",
        "git_head": git_output(root, "rev-parse", "HEAD").decode().strip(),
        "git_status": git_output(root, "status", "--short").decode("utf-8", errors="replace"),
    }


def validate_measurement(report, parameters, workload):
    if report.get("schema_version") != 1 or report.get("completed") is not True:
        raise ValueError("benchmark did not complete the supported report contract")
    if report.get("parameters") != parameters:
        raise ValueError("benchmark returned different parameters")
    expected = {"table-reinsert", "table-rotate", "closure-coroutine", "allocation-pressure"} if workload == "all" else {workload}
    results = report.get("workloads", [])
    if {result.get("name") for result in results} != expected or len(results) != len(expected):
        raise ValueError("benchmark did not report every selected workload exactly once")
    for result in results:
        if result.get("runtime_instances") != 1 or len(result.get("batches", [])) != parameters["batches"]:
            raise ValueError("benchmark Runtime/batch contract is incomplete")
        for key in ["execution_wall", "explicit_full_gc_wall"]:
            summary = result[key]
            if summary["samples"] != parameters["batches"] or not (summary["min_ns"] <= summary["p50_ns"] <= summary["p95_ns"] <= summary["p99_ns"] <= summary["max_ns"]):
                raise ValueError("invalid latency sample summary")


def write_report(path, report):
    path.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def main(argv=None):
    root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", choices=["dev", "release"], default="release")
    parser.add_argument("--target", help="Rust target triple; defaults to rustc host")
    parser.add_argument("--iterations", type=int, default=1000)
    parser.add_argument("--width", type=int, default=128)
    parser.add_argument("--batches", type=int, default=20)
    parser.add_argument("--warmup", type=int, default=3)
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--workload", choices=["all", "table-reinsert", "table-rotate", "closure-coroutine", "allocation-pressure"], default="all")
    parser.add_argument("--timeout-seconds", type=float, default=300)
    parser.add_argument("--build-timeout-seconds", type=float, default=900)
    parser.add_argument("--artifact", type=Path, default=root / "target/performance/runtime-baseline.json")
    args = parser.parse_args(argv)
    if min(args.iterations, args.width, args.batches, args.repeats, args.timeout_seconds, args.build_timeout_seconds) <= 0 or args.warmup < 0:
        parser.error("sizes, repeats, and timeouts must be positive; warmup must be nonnegative")
    artifact = args.artifact.resolve()
    work = artifact.parent / (artifact.stem + "-runs") / (datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-" + uuid.uuid4().hex[:8])
    work.mkdir(parents=True)
    parameters = {"iterations": args.iterations, "width": args.width, "batches": args.batches, "warmup": args.warmup}
    report = {
        "schema_version": 1, "complete": False, "passed": False,
        "started_utc": datetime.now(timezone.utc).isoformat(), "work_directory": str(work),
        "runner_sha256": digest(Path(__file__).read_bytes()),
        "process_helper_sha256": digest((root / TOOL_FILES[1]).read_bytes()),
        "parameters": parameters, "repeats": args.repeats, "workload_selection": args.workload,
        "host": {"platform": platform.platform(), "machine": platform.machine(), "processor": platform.processor(), "logical_cpus": os.cpu_count()},
        "interpretation": "Within-engine observations only. Host Instant timing and managed allocator accounting; not automatic GC pause measurements or OS RSS. No performance thresholds asserted.",
        "runs": [],
    }
    write_report(artifact, report)
    try:
        source = work / "sources"
        report["source"] = snapshot_source(root, source, work / "source.zip")
        rustc = subprocess.check_output(["rustc", "-vV"], cwd=source).decode("utf-8", errors="replace")
        report["rustc"] = rustc
        report["cargo"] = subprocess.check_output(["cargo", "--version"], cwd=source).decode().strip()
        target = args.target or next(line.removeprefix("host: ") for line in rustc.splitlines() if line.startswith("host: "))
        command = ["cargo", "build", "--locked", "-p", "lua_app", "--example", "runtime_baseline", "--profile", args.profile, "--target", target, "--target-dir", str(work / "build"), "--message-format=json"]
        report["build"] = {"command": command, "target": target, "profile": args.profile, "from_source_snapshot": True,
            "environment_overrides": {name: os.environ[name] for name in ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTC", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"] if name in os.environ}}
        write_report(artifact, report)
        print(f"Building {args.profile} example from frozen source {report['source']['manifest_sha256'][:12]}", flush=True)
        build = observe_process(command, source, args.build_timeout_seconds)
        report["build"]["observation"] = build
        if build["timed_out"] or build["exit_code"] != 0:
            raise ValueError(f"snapshot build failed; see {build['stderr']['path']}")
        if any(digest((source / name).read_bytes()) != sha for name, sha in report["source"]["files_sha256"].items()):
            raise ValueError("frozen source changed during build")
        executables = []
        for line in Path(build["stdout"]["path"]).read_text(encoding="utf-8").splitlines():
            record = json.loads(line)
            if record.get("reason") == "compiler-artifact" and record.get("target", {}).get("name") == "runtime_baseline" and record.get("executable"):
                executables.append(Path(record["executable"]))
        if len(executables) != 1:
            raise ValueError("Cargo did not identify exactly one benchmark executable")
        snapshots, identities = snapshot_engines({"runtime_baseline": executables[0]}, work / "engines")
        report["binary"] = identities["runtime_baseline"]
        report["binary"]["built_from_source_manifest_sha256"] = report["source"]["manifest_sha256"]
        for repetition in range(1, args.repeats + 1):
            if digest(snapshots["runtime_baseline"].read_bytes()) != report["binary"]["sha256"]:
                raise ValueError("benchmark binary snapshot changed between repetitions")
            directory = work / f"repeat-{repetition}"
            directory.mkdir()
            command = [str(snapshots["runtime_baseline"])]
            for name, value in parameters.items():
                command.extend(["--" + name, str(value)])
            command.extend(["--workload", args.workload])
            print(f"Running repetition {repetition}/{args.repeats} with binary {report['binary']['sha256'][:12]}", flush=True)
            result = {"repetition": repetition, "command": command, "status": "failed"}
            report["runs"].append(result)
            observed = observe_process(command, directory, args.timeout_seconds)
            result["observation"] = observed
            if observed["timed_out"]:
                result["status"] = "timeout"
            elif observed["exit_code"] not in (0, 1):
                result["status"] = "abnormal-exit"
            elif observed["exit_code"] == 0:
                try:
                    measurement = json.loads(Path(observed["stdout"]["path"]).read_text(encoding="utf-8"))
                    validate_measurement(measurement, parameters, args.workload)
                    result.update(status="passed", measurement=measurement)
                except (ValueError, KeyError, TypeError) as error:
                    result.update(status="invalid-report", error=str(error))
            write_report(artifact, report)
        report.update(complete=True, passed=all(result["status"] == "passed" for result in report["runs"]))
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        report["runner_error"] = str(error)
        write_report(artifact, report)
        print(str(error), file=sys.stderr)
        return 2
    write_report(artifact, report)
    print(f"Report: {artifact}")
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
