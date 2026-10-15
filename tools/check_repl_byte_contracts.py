#!/usr/bin/env python3
"""Check self-contained REPL corpus copies and optionally run both real binaries.

The oracle is permanent source data, never inferred from either executable.
Only a declared banner prefix differs. Captured streams are compared as bytes.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
CANONICAL = ROOT / "tools/repl_byte_contracts/cases.tsv"
SNAPSHOTS = {
    "v0.18": ROOT / "book/code/v18-cli-tools",
    "v1.0": ROOT / "book/code/v100-lua51-runtime",
}
BANNERS = {
    "v0.18": b"Lua 5.1 (lua-rust teaching v0.18)\n",
    "v1.0": b"Lua 5.1 (lua-rust teaching v1.0)\n",
}
COPY_PATHS = (
    "tests/fixtures/repl-byte-contract.tsv",
    "tests/repl_byte_contract.rs",
    "tests/support/repl_byte_contract.rs",
)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def decode_hex(field):
    if re.fullmatch(r"(?:[0-9a-f]{2})*", field) is None:
        raise ValueError("expected lowercase even-length hex field")
    return bytes.fromhex(field)


def parse_cases(raw):
    if not raw.endswith(b"\n"):
        raise ValueError("corpus must end in a newline")
    cases, ids = [], set()
    for line in raw.decode("ascii").splitlines():
        if line.startswith("#"):
            continue
        fields = line.split("|")
        if len(fields) != 7:
            raise ValueError("expected seven corpus columns")
        name, arguments, stdin, code, banner, stdout, stderr = fields
        if not re.fullmatch(r"[a-z]+(?:-[a-z]+)*", name) or name in ids:
            raise ValueError("invalid or duplicate case id: " + name)
        ids.add(name)
        args = [decode_hex(arg).decode("utf-8") for arg in arguments.split(",")] if arguments else []
        if code not in ("0", "1") or banner not in ("none", "repl"):
            raise ValueError("undeclared exit or banner mode")
        if (banner == "repl") != (args == ["-i"]):
            raise ValueError("only explicit interactive cases receive the banner")
        cases.append({
            "id": name, "args": args, "stdin": decode_hex(stdin),
            "exit": int(code), "banner": banner,
            "body": decode_hex(stdout), "stderr": decode_hex(stderr),
        })
    if len(cases) != 26:
        raise ValueError("expected the declared 26-case corpus")
    return cases


def check_sources():
    raw = CANONICAL.read_bytes()
    hashes = {str(CANONICAL.relative_to(ROOT)): digest(raw)}
    cases = parse_cases(raw)
    for relative in COPY_PATHS:
        left, right = [base / relative for base in SNAPSHOTS.values()]
        if left.read_bytes() != right.read_bytes():
            raise ValueError("snapshot copies differ: " + relative)
        for path in (left, right):
            hashes[str(path.relative_to(ROOT))] = digest(path.read_bytes())
        if relative.endswith(".tsv") and left.read_bytes() != raw:
            raise ValueError("snapshot fixture differs from canonical corpus")
    return cases, hashes


def write_new(path, data):
    with path.open("xb") as handle:
        handle.write(data)


def run_case(binary, version, case, directory, timeout):
    directory.mkdir()
    expected_stdout = (BANNERS[version] if case["banner"] == "repl" else b"") + case["body"]
    started = time.monotonic()
    process = subprocess.Popen(
        [str(binary), *case["args"]], stdin=subprocess.PIPE,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    )
    timed_out = False
    kill_error = None
    try:
        stdout, stderr = process.communicate(case["stdin"], timeout=timeout)
    except subprocess.TimeoutExpired:
        timed_out = True
        # Kill request and wait are distinct; communicate drains and waits afterward.
        try:
            process.kill()
        except OSError as error:
            kill_error = str(error)
        stdout, stderr = process.communicate()
    elapsed = time.monotonic() - started
    passed = (not timed_out and process.returncode == case["exit"]
              and stdout == expected_stdout and stderr == case["stderr"])
    for name, data in (
        ("stdin.bin", case["stdin"]), ("stdout.bin", stdout), ("stderr.bin", stderr),
        ("expected-stdout.bin", expected_stdout), ("expected-stderr.bin", case["stderr"]),
    ):
        write_new(directory / name, data)
    row = {
        "id": case["id"], "version": version, "executable": str(binary),
        "argv": case["args"], "exit": process.returncode, "expected_exit": case["exit"],
        "timed_out": timed_out, "reaped": process.returncode is not None,
        "kill_error": kill_error,
        "elapsed_seconds": elapsed, "passed": passed,
        "stdout_exact": stdout == expected_stdout, "stderr_exact": stderr == case["stderr"],
        "stdin_bytes": len(case["stdin"]), "stdout_bytes": len(stdout), "stderr_bytes": len(stderr),
    }
    write_new(directory / "metadata.json", json.dumps(row, ensure_ascii=False, indent=2).encode())
    print(version, case["id"], "PASS" if passed else "FAIL", flush=True)
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="check fixture/harness identity and schema")
    parser.add_argument("--v18-bin", type=Path)
    parser.add_argument("--v100-bin", type=Path)
    parser.add_argument("--output", type=Path, help="parent of a new unique evidence directory")
    parser.add_argument("--timeout", type=float, default=5.0, help="seconds per bounded child")
    args = parser.parse_args()
    if bool(args.v18_bin) != bool(args.v100_bin):
        parser.error("--v18-bin and --v100-bin must be supplied together")
    if args.timeout <= 0:
        parser.error("--timeout must be positive")
    cases, hashes = check_sources()
    print("source identity PASS; 26 fixed cases; only declared banner prefix varies", flush=True)
    if args.v18_bin is None:
        return 0
    binaries = {"v0.18": args.v18_bin.resolve(strict=True), "v1.0": args.v100_bin.resolve(strict=True)}
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ") + "-" + uuid.uuid4().hex[:8]
    evidence = (args.output or ROOT / "target/repl-byte-contracts") / stamp
    evidence.mkdir(parents=True, exist_ok=False)
    rows = []
    for version, binary in binaries.items():
        side = evidence / version
        side.mkdir()
        for case in cases:
            rows.append(run_case(binary, version, case, side / case["id"], args.timeout))
    summary = {
        "schema": "repl-byte-contract-v1", "cases_per_snapshot": len(cases),
        "all_passed": all(row["passed"] for row in rows), "sources": hashes,
        "binary_sha256": {version: digest(path.read_bytes()) for version, path in binaries.items()},
        "stream_policy": "raw byte equality; declared banner prefix in expectation only; no output normalization",
        "scope": "bounded children without descendants; injected host I/O errors belong to library tests",
        "cases": rows,
    }
    write_new(evidence / "summary.json", json.dumps(summary, ensure_ascii=False, indent=2).encode())
    print("EVIDENCE", evidence)
    return 0 if summary["all_passed"] else 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError) as error:
        print("contract error:", error, file=sys.stderr)
        sys.exit(1)
