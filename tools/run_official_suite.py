"""Execute the inventoried Lua 5.1 source suite in isolated child processes.

This is a strict observation lane, not an allow-failures gate: any executed
failure makes the command fail. Skipped files and upstream conditional skips
remain distinct from successful assertion workloads in its JSON report.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import time
import uuid


MARKER = "__OFFICIAL_SOURCE_COMPLETED__"
CAPTURE_LIMIT = 4 * 1024 * 1024


def digest(data):
    return hashlib.sha256(data).hexdigest()


def snapshot_engines(engines, destination):
    snapshots, identities = {}, {}
    for name, source in engines.items():
        directory = destination / name
        directory.mkdir(parents=True)
        binary = source.read_bytes()
        snapshot = directory / source.name
        snapshot.write_bytes(binary)
        shutil.copymode(source, snapshot)
        dependencies = {}
        # Windows developer builds may keep runtime DLLs beside the executable.
        for dependency in source.parent.glob("*.dll"):
            data = dependency.read_bytes()
            (directory / dependency.name).write_bytes(data)
            dependencies[dependency.name] = digest(data)
        snapshots[name] = snapshot
        identities[name] = {"input_path": str(source), "snapshot_path": str(snapshot), "sha256": digest(binary), "adjacent_dll_sha256": dependencies}
    return snapshots, identities


def load_inventory(root, manifest_path):
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if manifest.get("schema_version") != 1:
        raise ValueError("unsupported official source lane schema")
    source = root / manifest["source_root"]
    provenance = json.loads((root / manifest["source_provenance"]).read_text(encoding="utf-8"))
    pinned = {entry["path"]: entry["sha256"] for entry in provenance["entries"]}
    names = [case["file"] for case in manifest["cases"]]
    actual = {path.name for path in source.glob("*.lua")}
    pinned_lua = {name for name in pinned if name.endswith(".lua") and "/" not in name}
    if len(names) != len(set(names)) or set(names) != actual or actual != pinned_lua:
        raise ValueError("official lane inventory must classify each pinned source exactly once")
    hashes = {}
    manifest["unavailable_provenance_assets"] = sorted(name for name in pinned if not (source / name).is_file())
    for case in manifest["cases"]:
        if case["role"] not in {"run", "helper", "excluded"} or not case.get("reason"):
            raise ValueError(f"missing classification/reason: {case['file']}")
        raw = (source / case["file"]).read_bytes()
        # Git's Windows checkout may use CRLF. Verify provenance on the Git
        # newline form, but execute exact working-tree bytes without rewrites.
        canonical_hash = digest(raw.replace(b"\r\n", b"\n"))
        if canonical_hash != pinned[case["file"]]:
            raise ValueError(f"source provenance mismatch: {case['file']}")
        hashes[case["file"]] = {"working_tree_sha256": digest(raw), "upstream_lf_sha256": canonical_hash}
    return manifest, source, hashes


def bootstrap(case):
    checks = ""
    if "expected_return" in case:
        checks = f"assert(result == {case['expected_return']}, 'upstream return contract')\n"
    elif "expected_return_global" in case:
        name = case["expected_return_global"]
        if not re.fullmatch(r"[A-Za-z_][A-Za-z_0-9]*", name):
            raise ValueError("invalid return-contract global")
        checks = f"assert(result == {name} and {name}, 'upstream return contract')\n"
    filename = json.dumps(case["file"])
    return (
        "local finish = print\n"
        "math.randomseed(0)\n"
        "collectgarbage('setstepmul', 180)\n"
        "collectgarbage('setpause', 190)\n"
        "assert(os.setlocale('C'))\n"
        "package.path = '?;./?.lua'\n"
        "assert(T == nil and _soft == nil, 'source profile requires stock non-soft runtime')\n"
        "function Message(message) print(message) end\n"
        f"local result = assert(loadfile({filename}))()\n"
        + checks
        + f"finish('{MARKER}:{case['file']}')\n"
    )


def terminate_tree(process):
    if os.name == "nt":
        # taskkill receives one observed PID, not a generated shell command.
        subprocess.run(
            ["taskkill", "/F", "/T", "/PID", str(process.pid)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
            creationflags=subprocess.CREATE_NO_WINDOW,
        )
    else:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)


def observe_process(command, cwd, timeout):
    stdout_path, stderr_path = cwd / "stdout.bin", cwd / "stderr.bin"
    start = time.monotonic()
    timed_out = False
    environment = dict(os.environ)
    # Upstream README requires the bare '?' path for main.lua's '-l <temp>'
    # subprocesses. Set it in their inherited environment as well as Lua.
    environment["LUA_PATH"] = "?;./?.lua"
    environment.pop("LUA_INIT", None)
    temporary = cwd / "tmp"
    temporary.mkdir(exist_ok=True)
    for name in ("TMP", "TEMP", "TMPDIR"):
        environment[name] = str(temporary)
    with stdout_path.open("wb") as stdout, stderr_path.open("wb") as stderr:
        kwargs = {"creationflags": subprocess.CREATE_NO_WINDOW | subprocess.CREATE_NEW_PROCESS_GROUP} if os.name == "nt" else {"start_new_session": True}
        process = subprocess.Popen(command, cwd=cwd, env=environment, stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr, **kwargs)
        try:
            code = process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            terminate_tree(process)
            code = process.returncode
        except BaseException:
            terminate_tree(process)
            raise
    streams = {}
    for name, path in [("stdout", stdout_path), ("stderr", stderr_path)]:
        with path.open("rb") as stream:
            raw = stream.read(CAPTURE_LIMIT)
        streams[name] = {"path": str(path), "size": path.stat().st_size, "truncated": path.stat().st_size > CAPTURE_LIMIT, "hex": raw.hex(), "text": raw.decode("utf-8", errors="replace")}
    return {"exit_code": code, "timed_out": timed_out, "duration_seconds": round(time.monotonic() - start, 3), **streams}


def classify(observation, filename):
    if observation["timed_out"]:
        return "timeout"
    code = observation["exit_code"]
    if code not in (0, 1):
        return "abnormal-exit"
    if code != 0:
        return "failed"
    marker = f"{MARKER}:{filename}"
    if marker not in observation["stdout"]["text"].splitlines():
        return "missing-completion"
    return "passed"


def run_case(case, engine, executable, source, destination, timeout):
    work = destination / engine / Path(case["file"]).stem
    work.mkdir(parents=True)
    for path in source.glob("*.lua"):
        shutil.copyfile(path, work / path.name)
    (work / "libs/P1").mkdir(parents=True)
    launcher = work / "official-source-bootstrap.lua"
    launcher.write_text(bootstrap(case), encoding="utf-8", newline="\n")
    command = [str(executable), launcher.name]
    try:
        observation = observe_process(command, work, timeout)
    except OSError as error:
        return {"status": "runner-error", "error": str(error), "command": command, "working_directory": str(work)}
    output = observation["stdout"]["text"] + "\n" + observation["stderr"]["text"]
    observation.update({
        "status": classify(observation, case["file"]),
        "command": command,
        "working_directory": str(work),
        "temporary_directory": str(work / "tmp"),
        "upstream_skip_messages": re.findall(r">>>\s*(.*?)\s*<<<", output, re.DOTALL),
    })
    match = re.search(r"(?:^|[ /\\])" + re.escape(case["file"]) + r":(\d+):([^\n]*)", output)
    if match:
        line = int(match[1])
        source_lines = (source / case["file"]).read_bytes().decode("latin1").splitlines()
        observation["failure_location"] = {"file": case["file"], "line": line, "message": match[2].strip(), "source": source_lines[line - 1] if 0 < line <= len(source_lines) else None}
    return observation


def main(argv=None):
    root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=root / "tests/compatibility/official-source-lane.json")
    parser.add_argument("--rust", type=Path, required=True)
    parser.add_argument("--official", type=Path, required=True)
    parser.add_argument("--cpp", type=Path)
    parser.add_argument("--artifact", type=Path, default=root / "target/compatibility/official-source-lane.json")
    parser.add_argument("--case", action="append", default=[])
    parser.add_argument("--timeout-seconds", type=float, help="override every per-file timeout")
    args = parser.parse_args(argv)
    if args.timeout_seconds is not None and args.timeout_seconds <= 0:
        parser.error("timeout must be positive")
    manifest, source, hashes = load_inventory(root, args.manifest.resolve())
    selected = set(args.case)
    unknown = selected - {case["file"] for case in manifest["cases"]}
    if unknown:
        parser.error(f"unknown cases: {sorted(unknown)}")
    not_runnable = selected - {case["file"] for case in manifest["cases"] if case["role"] == "run"}
    if not_runnable:
        parser.error(f"explicitly selected cases must run, not be skipped: {sorted(not_runnable)}")
    engines = {"official_lua51": args.official.resolve(), "lua_rust": args.rust.resolve()}
    if args.cpp:
        engines["cpp_pinned"] = args.cpp.resolve()
    for executable in engines.values():
        if not executable.is_file():
            parser.error(f"missing executable: {executable}")
    artifact = args.artifact.resolve()
    work = artifact.parent / (artifact.stem + "-runs") / (time.strftime("%Y%m%dT%H%M%S") + "-" + uuid.uuid4().hex[:8])
    work.mkdir(parents=True)
    source_input = source
    source = work / "sources"
    source.mkdir()
    for name, expected in hashes.items():
        raw = (source_input / name).read_bytes()
        if digest(raw) != expected["working_tree_sha256"]:
            raise ValueError(f"source changed during snapshot: {name}")
        (source / name).write_bytes(raw)
    engines, identities = snapshot_engines(engines, work / "engines")
    report = {"schema_version": 1, "profile": manifest["profile"], "scope": manifest["scope"], "manifest": str(args.manifest.resolve()), "runner_sha256": digest(Path(__file__).read_bytes()), "source_input": str(source_input), "source_snapshot": str(source), "environment": {"LUA_PATH": "?;./?.lua", "LUA_INIT": None, "TMP/TEMP/TMPDIR": "per-case tmp directory"}, "unavailable_provenance_assets": manifest["unavailable_provenance_assets"], "source_hashes": hashes, "engines": identities, "selection": sorted(selected), "complete": False, "passed": False, "cases": []}
    artifact.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    for case in manifest["cases"]:
        record = dict(case)
        record["executions"] = {}
        if case["role"] == "run" and (not selected or case["file"] in selected):
            for engine, executable in engines.items():
                observation = run_case(case, engine, executable, source, work, args.timeout_seconds or case.get("timeout_seconds", 30))
                record["executions"][engine] = observation
                print(f"{case['file']:14} {engine:16} {observation['status']}", flush=True)
        elif selected and case["file"] not in selected:
            record["selection_status"] = "not-selected"
        report["cases"].append(record)
        artifact.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    summaries = {}
    for engine in engines:
        observations = [case["executions"][engine] for case in report["cases"] if engine in case["executions"]]
        summaries[engine] = {"executed": len(observations), "passed": sum(item["status"] == "passed" for item in observations), "failed": sum(item["status"] != "passed" for item in observations)}
    report["summary"] = summaries
    report["complete"] = True
    report["passed"] = all(summary["executed"] > 0 and summary["failed"] == 0 for summary in summaries.values())
    report["complete_upstream_suite"] = False
    artifact.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(summaries), flush=True)
    print(f"artifact: {artifact}", flush=True)
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, KeyError) as error:
        print(f"official source lane: {error}", file=sys.stderr)
        sys.exit(2)
