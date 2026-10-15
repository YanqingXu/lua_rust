#!/usr/bin/env python3
"""Verify the fixed bounded Lua-5.1 string.find oracle without output normalization."""
from __future__ import annotations

import argparse
import csv
from datetime import datetime, timezone
import hashlib
import io
import json
from pathlib import Path
import platform
import re
import struct
import subprocess
import sys
import tarfile
import time
import uuid

SNAPSHOT = Path(__file__).resolve().parents[1]
FIXTURE = SNAPSHOT / "tests/fixtures/string-find/cases.tsv"
DRIVER = SNAPSHOT / "tests/fixtures/string-find/driver.lua"
SOURCE_SHA256 = "2640fc56a795f29d28ef15e13c34a47e223960b0240e8cb0a82d9b0738695333"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def corpus():
    raw = FIXTURE.read_bytes()
    if not raw.endswith(b"\n"):
        raise ValueError("fixture must end in a newline")
    lines = [line for line in raw.decode("ascii").splitlines() if not line.startswith("#")]
    rows = list(csv.DictReader(lines, delimiter="\t"))
    if not lines or lines[0] != "id\tresult\tarity\tvalues" or len(rows) != 75:
        raise ValueError("expected declared 75-row fixture schema")
    ids = set()
    for row in rows:
        if set(row) != {"id", "result", "arity", "values"}:
            raise ValueError("unexpected fixture columns")
        if not re.fullmatch(r"[a-z]+(?:-[a-z]+)*", row["id"]) or row["id"] in ids:
            raise ValueError("bad or duplicate case id")
        ids.add(row["id"])
        if row["result"] == "error":
            if (row["arity"], row["values"]) != ("-", "-"):
                raise ValueError("error rows use explicit '-' placeholders")
        elif row["result"] == "ok":
            values = row["values"].split(",")
            if row["arity"] != str(len(values)):
                raise ValueError("declared arity differs from fixed values")
            if not (values == ["nil"] or (len(values) == 2 and all(re.fullmatch(r"n:(?:0|[1-9][0-9]*)", value) for value in values))):
                raise ValueError("expected exactly one nil or two byte coordinates")
        else:
            raise ValueError("unknown fixed result class")
    selected = re.findall(r'\["([a-z-]+)"\]\s*=\s*function', DRIVER.read_text(encoding="utf-8"))
    if selected != [row["id"] for row in rows]:
        raise ValueError("driver case IDs/order differ from fixed oracle")
    return rows


def expected_stdout(case):
    if case["result"] == "ok":
        return ("ok|" + case["arity"] + "|" + case["values"].replace(",", "|") + "|after").encode("ascii")
    return b"error|after"


def write_new(path, data):
    with path.open("xb") as output:
        output.write(data)


def capture(binary, arguments, timeout):
    started = time.monotonic()
    process = subprocess.Popen(
        [str(binary), *arguments], stdin=subprocess.PIPE,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    )
    timed_out, kill_error = False, None
    try:
        stdout, stderr = process.communicate(b"", timeout=timeout)
    except subprocess.TimeoutExpired:
        timed_out = True
        try:
            process.kill()
        except OSError as error:
            kill_error = str(error)
        stdout, stderr = process.communicate()  # drain both streams and wait, even after a kill race
    return {
        "exit": process.returncode, "stdout": stdout, "stderr": stderr,
        "timed_out": timed_out, "kill_error": kill_error,
        "reaped": process.returncode is not None,
        "elapsed_seconds": time.monotonic() - started,
    }


def binary_identity(path):
    data = path.read_bytes()
    machine = None
    if data[:2] == b"MZ":
        pe = struct.unpack_from("<I", data, 0x3C)[0]
        if data[pe:pe + 4] != b"PE\0\0":
            raise ValueError("invalid PE signature")
        machine = hex(struct.unpack_from("<H", data, pe + 4)[0])
    return {"path": str(path), "sha256": sha(data), "bytes": len(data), "pe_machine": machine}


def official_source(archive):
    raw = archive.read_bytes()
    if sha(raw) != SOURCE_SHA256:
        raise ValueError("official source archive SHA-256 does not match pinned Lua 5.1.5")
    files = {}
    with tarfile.open(fileobj=io.BytesIO(raw), mode="r:gz") as source:
        for name in ("src/lstrlib.c", "src/luaconf.h", "src/lua.h"):
            member = source.extractfile("lua-5.1.5/" + name)
            if member is None:
                raise ValueError("missing official source member: " + name)
            files[name] = sha(member.read())
    return {"archive": str(archive), "archive_sha256": sha(raw), "member_sha256": files}


def execute_case(binary, role, case, output, timeout):
    directory = output / role / case["id"]
    directory.mkdir(parents=True, exist_ok=False)
    actual = capture(binary, [str(DRIVER), case["id"]], timeout)
    expected = expected_stdout(case)
    passed = (not actual["timed_out"] and actual["reaped"] and actual["exit"] == 0
              and actual["stdout"] == expected and actual["stderr"] == b"")
    for name, data in (
        ("stdin.bin", b""), ("stdout.bin", actual["stdout"]), ("stderr.bin", actual["stderr"]),
        ("expected-stdout.bin", expected), ("expected-stderr.bin", b""),
    ):
        write_new(directory / name, data)
    row = {
        key: value for key, value in actual.items() if key not in ("stdout", "stderr")
    }
    row.update({
        "role": role, "id": case["id"], "passed": passed, "expected_exit": 0,
        "argv": [str(DRIVER), case["id"]], "stdin_bytes": 0,
        "stdout_bytes": len(actual["stdout"]), "stderr_bytes": len(actual["stderr"]),
        "stdout_sha256": sha(actual["stdout"]), "stderr_sha256": sha(actual["stderr"]),
        "stdout_exact": actual["stdout"] == expected, "stderr_exact": actual["stderr"] == b"",
    })
    write_new(directory / "metadata.json", json.dumps(row, ensure_ascii=False, indent=2).encode())
    print(role, case["id"], "PASS" if passed else "FAIL", flush=True)
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="schema/driver check only; no runtime compatibility claim")
    parser.add_argument("--candidate-bin", type=Path)
    parser.add_argument("--official-bin", type=Path)
    parser.add_argument("--official-archive", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--timeout", type=float, default=5.0)
    arguments = parser.parse_args()
    provided = (arguments.candidate_bin, arguments.official_bin, arguments.official_archive)
    if any(provided) and not all(provided):
        parser.error("actual comparison requires --candidate-bin, --official-bin and --official-archive together")
    if arguments.timeout <= 0:
        parser.error("--timeout must be positive")
    cases = corpus()
    print("schema PASS: 75 fixed cases; raw byte capture; arity/coordinates are encoded inside Lua", flush=True)
    if not any(provided):
        print("candidate NOT_RUN; official NOT_RUN (schema check only)", flush=True)
        return 0
    candidate = arguments.candidate_bin.resolve(strict=True)
    official = arguments.official_bin.resolve(strict=True)
    source = official_source(arguments.official_archive.resolve(strict=True))
    version = capture(official, ["-v"], arguments.timeout)
    if (version["timed_out"] or version["exit"] != 0
            or b"Lua 5.1.5" not in version["stdout"] + version["stderr"]):
        raise ValueError("official executable did not identify itself as Lua 5.1.5")
    identity = {"candidate": binary_identity(candidate), "official": binary_identity(official)}
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ") + "-" + uuid.uuid4().hex[:8]
    output = (arguments.output or SNAPSHOT / "target/string-find-contract") / stamp
    output.mkdir(parents=True, exist_ok=False)
    write_new(output / "official-version.stdout.bin", version["stdout"])
    write_new(output / "official-version.stderr.bin", version["stderr"])
    write_new(output / "cases.tsv", FIXTURE.read_bytes())
    write_new(output / "driver.lua", DRIVER.read_bytes())
    rows = []
    for role, binary in (("candidate", candidate), ("official", official)):
        for case in cases:
            rows.append(execute_case(binary, role, case, output, arguments.timeout))
    summary = {
        "schema": "string-find-contract/v1", "created_utc": stamp,
        "all_passed": all(row["passed"] for row in rows), "cases_per_binary": 75,
        "candidate_status": "passed" if all(row["passed"] for row in rows if row["role"] == "candidate") else "failed",
        "official_status": "passed" if all(row["passed"] for row in rows if row["role"] == "official") else "failed",
        "binary_identity": identity, "official_source": source,
        "official_version": {key: value for key, value in version.items() if key not in ("stdout", "stderr")},
        "input_sha256": {
            "cases.tsv": sha(FIXTURE.read_bytes()), "driver.lua": sha(DRIVER.read_bytes()),
            "checker.py": sha(Path(__file__).read_bytes()),
        },
        "platform": {"system": platform.platform(), "python": platform.python_version(), "libc": platform.libc_ver()},
        "stream_policy": "full raw stdout/stderr and exit; no trim, decode or CRLF normalization",
        "lua_transport": "pcall result/error, exact return arity, nil or byte coordinates and after sentinel; ASCII arithmetic/char encoding inside Lua without a formatter",
        "error_scope": "pcall false is stable across engines; local Rust tests separately require VM Type, argument number, malformed category and reached-vs-unreached validation timing",
        "bounded_scope": "plain/init, byte atoms/classes/sets, anchors and greedy */+; no optional/lazy quantifiers, captures/backrefs, balance/frontier, gmatch/gsub, locale or universal complexity guarantee",
        "candidate_policy_scope": "Unsupported and bounded step/backtrack Limit are separate candidate-only tests, never mislabeled as official equivalent results",
        "provenance_scope": "source archive and executable identities are recorded separately; version text alone is not a reproducible build proof",
        "cases": rows,
    }
    write_new(output / "summary.json", json.dumps(summary, ensure_ascii=False, indent=2).encode())
    print("EVIDENCE", output, flush=True)
    return 0 if summary["all_passed"] else 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, tarfile.TarError) as error:
        print("find contract error:", error, file=sys.stderr)
        sys.exit(1)
