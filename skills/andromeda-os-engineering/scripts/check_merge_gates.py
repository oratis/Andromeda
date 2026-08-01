#!/usr/bin/env python3
"""Check exact-head GitHub merge gates for an Andromeda pull request."""

from __future__ import annotations

import argparse
import fnmatch
import json
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any


BASE_REQUIRED = {
    "Format, lint, and test",
    "Platform probe (ubuntu-latest)",
    "Platform probe (macos-latest)",
    "Platform probe (windows-latest)",
}
OS_E2E_CHECK = "UEFI install, first boot, update, and rollback"
OS_E2E_PATTERNS = (
    ".github/workflows/os-e2e.yml",
    "Cargo.lock",
    "Cargo.toml",
    "crates/**",
    "os/**",
)
PASSING_CONCLUSIONS = {"SUCCESS"}


@dataclass(frozen=True)
class GateResult:
    ready: bool
    pending: bool
    reasons: tuple[str, ...]
    payload: dict[str, Any]


def run(command: list[str], cwd: Path) -> str:
    result = subprocess.run(
        command,
        cwd=cwd,
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if result.returncode != 0:
        message = result.stderr.strip() or result.stdout.strip()
        raise RuntimeError(f"{' '.join(command)} failed: {message}")
    return result.stdout


def matches_os_e2e(path: str) -> bool:
    return any(fnmatch.fnmatchcase(path, pattern) for pattern in OS_E2E_PATTERNS)


def evaluate(data: dict[str, Any]) -> GateResult:
    reasons: list[str] = []
    pending = False

    if data["state"] not in {"OPEN", "MERGED"}:
        reasons.append(f"PR state is {data['state']}")
    if data["state"] == "OPEN" and data["isDraft"]:
        reasons.append("PR is still a draft")
    if data["state"] == "OPEN":
        mergeable = data.get("mergeable")
        if mergeable == "UNKNOWN":
            pending = True
            reasons.append("GitHub mergeability is still unknown")
        elif mergeable != "MERGEABLE":
            reasons.append(f"PR is not mergeable: {mergeable}")

    paths = [entry["path"] for entry in data.get("files", [])]
    e2e_required = any(matches_os_e2e(path) for path in paths)
    required = set(BASE_REQUIRED)
    if e2e_required:
        required.add(OS_E2E_CHECK)

    checks: dict[str, dict[str, Any]] = {}
    for check in data.get("statusCheckRollup", []):
        name = check.get("name")
        if name:
            checks[name] = check

    missing = sorted(required - checks.keys())
    if missing:
        pending = True
        reasons.append(f"required checks missing: {', '.join(missing)}")

    for name in sorted(required & checks.keys()):
        check = checks[name]
        status = (check.get("status") or "").upper()
        conclusion = (check.get("conclusion") or "").upper()
        if status != "COMPLETED":
            pending = True
            reasons.append(f"{name} is {status or 'PENDING'}")
        elif conclusion not in PASSING_CONCLUSIONS:
            reasons.append(f"{name} concluded {conclusion or 'UNKNOWN'}")

    payload = {
        "number": data["number"],
        "url": data["url"],
        "state": data["state"],
        "head_sha": data["headRefOid"],
        "mergeable": data.get("mergeable"),
        "merge_state_status": data.get("mergeStateStatus"),
        "os_e2e_required": e2e_required,
        "changed_files": paths,
        "required_checks": sorted(required),
        "checks": {
            name: {
                "status": checks[name].get("status"),
                "conclusion": checks[name].get("conclusion"),
                "url": checks[name].get("detailsUrl"),
            }
            for name in sorted(required & checks.keys())
        },
    }
    ready = not reasons
    return GateResult(ready=ready, pending=pending and not ready, reasons=tuple(reasons), payload=payload)


def load_pr(number: int, repository: str | None, cwd: Path) -> dict[str, Any]:
    command = [
        "gh",
        "pr",
        "view",
        str(number),
        "--json",
        "number,state,isDraft,headRefOid,mergeable,mergeStateStatus,files,statusCheckRollup,url",
    ]
    if repository:
        command.extend(["--repo", repository])
    return json.loads(run(command, cwd))


def print_human(result: GateResult) -> None:
    state = "READY" if result.ready else "PENDING" if result.pending else "BLOCKED"
    payload = result.payload
    print(f"ANDROMEDA_MERGE_GATE={state}")
    print(f"PR={payload['number']}")
    print(f"URL={payload['url']}")
    print(f"HEAD_SHA={payload['head_sha']}")
    print(f"OS_E2E_REQUIRED={str(payload['os_e2e_required']).lower()}")
    for name, check in payload["checks"].items():
        print(
            f"CHECK={name} status={check['status']} conclusion={check['conclusion']} "
            f"url={check['url']}"
        )
    for reason in result.reasons:
        print(f"REASON={reason}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pr", type=int, help="pull request number")
    parser.add_argument("--repo", help="GitHub owner/repository override")
    parser.add_argument("--path", default=".", help="checkout path used by gh")
    parser.add_argument("--json", action="store_true", help="emit JSON")
    args = parser.parse_args()

    if shutil.which("gh") is None:
        print("ANDROMEDA_MERGE_GATE_ERROR gh-not-found", file=sys.stderr)
        return 1

    try:
        result = evaluate(load_pr(args.pr, args.repo, Path(args.path).resolve()))
    except (OSError, RuntimeError, json.JSONDecodeError, KeyError) as error:
        print(f"ANDROMEDA_MERGE_GATE_ERROR {error}", file=sys.stderr)
        return 1

    if args.json:
        output = dict(result.payload)
        output.update(
            {
                "ready": result.ready,
                "pending": result.pending,
                "reasons": list(result.reasons),
            }
        )
        print(json.dumps(output, indent=2, sort_keys=True))
    else:
        print_human(result)

    if result.ready:
        return 0
    return 2 if result.pending else 1


if __name__ == "__main__":
    raise SystemExit(main())
