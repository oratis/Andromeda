#!/usr/bin/env python3
"""Read-only Git and GitHub state audit for an Andromeda checkout."""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any


def run(command: list[str], cwd: Path, check: bool = True) -> str:
    result = subprocess.run(
        command,
        cwd=cwd,
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if check and result.returncode != 0:
        message = result.stderr.strip() or result.stdout.strip()
        raise RuntimeError(f"{' '.join(command)} failed: {message}")
    return result.stdout.strip()


def optional(command: list[str], cwd: Path) -> str | None:
    try:
        value = run(command, cwd)
    except RuntimeError:
        return None
    return value or None


def github_state(root: Path, repository: str | None) -> dict[str, Any]:
    if shutil.which("gh") is None:
        return {"available": False, "reason": "gh-not-found"}

    repo = repository or optional(
        ["gh", "repo", "view", "--json", "nameWithOwner", "--jq", ".nameWithOwner"],
        root,
    )
    if repo is None:
        return {"available": False, "reason": "gh-repository-unavailable"}

    open_prs_raw = optional(
        [
            "gh",
            "pr",
            "list",
            "--repo",
            repo,
            "--state",
            "open",
            "--limit",
            "100",
            "--json",
            "number,title,headRefName,baseRefName,isDraft,url",
        ],
        root,
    )
    runs_raw = optional(
        [
            "gh",
            "run",
            "list",
            "--repo",
            repo,
            "--limit",
            "8",
            "--json",
            "databaseId,workflowName,status,conclusion,headSha,event,url,createdAt",
        ],
        root,
    )
    return {
        "available": True,
        "repository": repo,
        "open_prs": json.loads(open_prs_raw) if open_prs_raw else [],
        "recent_runs": json.loads(runs_raw) if runs_raw else [],
    }


def collect(root_arg: str, repository: str | None, include_github: bool) -> dict[str, Any]:
    candidate = Path(root_arg).expanduser().resolve()
    root_text = run(["git", "rev-parse", "--show-toplevel"], candidate)
    root = Path(root_text)
    porcelain = run(["git", "status", "--porcelain=v1"], root)

    data: dict[str, Any] = {
        "root": str(root),
        "branch": optional(["git", "branch", "--show-current"], root),
        "head": run(["git", "rev-parse", "HEAD"], root),
        "worktree_clean": not bool(porcelain),
        "worktree_changes": porcelain.splitlines() if porcelain else [],
        "origin_main": optional(["git", "rev-parse", "origin/main"], root),
        "upstream": optional(
            ["git", "rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"],
            root,
        ),
    }
    if include_github:
        data["github"] = github_state(root, repository)
    return data


def print_human(data: dict[str, Any]) -> None:
    print(f"REPO_ROOT={data['root']}")
    print(f"BRANCH={data.get('branch') or 'DETACHED'}")
    print(f"HEAD={data['head']}")
    print(f"ORIGIN_MAIN={data.get('origin_main') or 'UNAVAILABLE'}")
    print(f"UPSTREAM={data.get('upstream') or 'NONE'}")
    print(f"WORKTREE_CLEAN={str(data['worktree_clean']).lower()}")
    for change in data["worktree_changes"]:
        print(f"WORKTREE_CHANGE={change}")

    github = data.get("github")
    if not github:
        return
    print(f"GITHUB_AVAILABLE={str(github['available']).lower()}")
    if not github["available"]:
        print(f"GITHUB_REASON={github['reason']}")
        return
    print(f"GITHUB_REPOSITORY={github['repository']}")
    print(f"OPEN_PRS={len(github['open_prs'])}")
    for pr in github["open_prs"]:
        print(
            "PR="
            f"{pr['number']} draft={str(pr['isDraft']).lower()} "
            f"{pr['headRefName']}->{pr['baseRefName']} {pr['url']}"
        )
    for workflow_run in github["recent_runs"]:
        conclusion = workflow_run.get("conclusion") or "pending"
        print(
            "RUN="
            f"{workflow_run['databaseId']} {workflow_run['workflowName']} "
            f"{workflow_run['status']}/{conclusion} {workflow_run['headSha']} "
            f"{workflow_run['url']}"
        )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("path", nargs="?", default=".", help="path inside the checkout")
    parser.add_argument("--repo", help="GitHub owner/repository override")
    parser.add_argument(
        "--github",
        action="store_true",
        help="include read-only GitHub PR and recent Actions state",
    )
    parser.add_argument("--json", action="store_true", help="emit JSON")
    args = parser.parse_args()

    try:
        data = collect(args.path, args.repo, args.github)
    except (OSError, RuntimeError, json.JSONDecodeError) as error:
        print(f"ANDROMEDA_REPOSITORY_AUDIT_ERROR {error}", file=sys.stderr)
        return 1

    if args.json:
        print(json.dumps(data, indent=2, sort_keys=True))
    else:
        print_human(data)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
