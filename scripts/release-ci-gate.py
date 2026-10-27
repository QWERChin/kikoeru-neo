#!/usr/bin/env python3
"""Require successful main-branch CI for the code in the checked-out release tag."""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request


class GateError(RuntimeError):
    pass


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, url):
        raise GateError(f"Actions API returned redirect {code}; refusing to follow it")


class GitHub:
    def __init__(self, repo, token):
        self.base = f"https://api.github.com/repos/{repo}"
        self.token = token
        self.opener = urllib.request.build_opener(NoRedirect())

    def request(self, path):
        request = urllib.request.Request(self.base + path, headers={
            "Authorization": f"Bearer {self.token}",
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
        })
        try:
            with self.opener.open(request, timeout=30) as response:
                if response.status != 200:
                    raise GateError(f"Actions API returned HTTP {response.status}")
                return json.loads(response.read())
        except (urllib.error.URLError, OSError, ValueError) as error:
            raise GateError(f"Actions API request failed for {path}: {error}") from error


def git(root, *args):
    result = subprocess.run(["git", *args], cwd=root, capture_output=True)
    if result.returncode:
        raise GateError(f"git {' '.join(args)} failed: {os.fsdecode(result.stderr).strip()}")
    return os.fsdecode(result.stdout)


def mapping(lines, indent):
    # Parse only the mapping levels that locate on.push.paths-ignore.
    # Reject aliases, quoted keys and duplicates instead of guessing YAML semantics.
    entries = {}
    key = None
    for line in lines:
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        if "\t" in line:
            raise GateError("unsupported paths-ignore form: tabs in workflow mapping")
        depth = len(line) - len(line.lstrip(" "))
        if depth > indent and key is not None:
            entries[key][1].append(line)
            continue
        match = re.fullmatch(r" {" + str(indent) + r"}([A-Za-z_][A-Za-z0-9_-]*):(?: (.*))?", line)
        if not match or match[1] in entries:
            raise GateError(f"unsupported paths-ignore form: workflow mapping at {line!r}")
        key = match[1]
        entries[key] = (match[2] or "", [])
    return entries


def ignored_patterns(workflow):
    lines = workflow.splitlines()
    for key, indent in [("on", 0), ("push", 2)]:
        entries = mapping(lines, indent)
        if key not in entries or entries[key][0]:
            raise GateError(f"unsupported paths-ignore form: expected a block for {key}")
        lines = entries[key][1]
    entries = mapping(lines, 4)
    if "paths-ignore" not in entries or entries["paths-ignore"][1]:
        raise GateError("unsupported paths-ignore form: expected a one-line paths-ignore list")
    try:
        patterns = json.loads(entries["paths-ignore"][0])
    except ValueError as error:
        raise GateError("unsupported paths-ignore form: expected a one-line double-quoted list") from error
    if not isinstance(patterns, list) or any(not isinstance(p, str) for p in patterns):
        raise GateError("unsupported paths-ignore form: expected a list of strings")
    for pattern in patterns:
        literal = pattern.removesuffix("/**")
        if (not re.fullmatch(r"[A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.-]+)*", literal)
                or any(part in {".", ".."} for part in literal.split("/"))):
            raise GateError(f"unsupported paths-ignore glob: {pattern!r}")
    return patterns


def validate_run(run, sha):
    if not isinstance(run, dict):
        raise GateError("unexpected Actions JSON: run is not an object")
    required = {"id", "head_sha", "head_branch", "event", "path", "status", "conclusion", "html_url"}
    if not required <= run.keys():
        raise GateError("unexpected Actions JSON: missing run fields")
    if (type(run["id"]) is not int or run["id"] <= 0 or run["head_sha"] != sha
            or run["path"] != ".github/workflows/ci.yml"
            or not isinstance(run["head_branch"], (str, type(None)))
            or not isinstance(run["event"], str)
            or not isinstance(run["html_url"], str) or not run["html_url"].startswith("https://")):
        raise GateError("unexpected Actions JSON: invalid run identity")
    status, conclusion = run["status"], run["conclusion"]
    pending = ("queued", "in_progress", "requested", "waiting", "pending")
    completed = ("success", "failure", "cancelled", "timed_out", "action_required",
                 "neutral", "skipped", "stale", "startup_failure")
    if not ((status == "completed" and conclusion in completed)
            or (status in pending and conclusion is None)):
        raise GateError("unexpected Actions JSON: invalid run status or conclusion")


def runs_for_commit(api, sha, check_time):
    runs, ids = [], set()
    total = None
    page = 1
    while True:
        check_time()
        query = urllib.parse.urlencode({"head_sha": sha, "per_page": 100, "page": page})
        data = api.request(f"/actions/workflows/ci.yml/runs?{query}")
        check_time()
        if (not isinstance(data, dict) or type(data.get("total_count")) is not int
                or data["total_count"] < 0 or not isinstance(data.get("workflow_runs"), list)):
            raise GateError("unexpected Actions JSON: expected total_count and workflow_runs")
        # Filtered workflow-run queries expose at most 1,000 results. At the
        # cap we cannot prove completeness, even if an earlier page passed.
        if data["total_count"] >= 1000:
            raise GateError("Actions pagination reached the 1,000-run limit")
        if total is None:
            total = data["total_count"]
        batch = data["workflow_runs"]
        if data["total_count"] != total or len(batch) != min(100, total - len(runs)):
            raise GateError("inconsistent Actions pagination; refusing an incomplete run list")
        for run in batch:
            validate_run(run, sha)
            if run["id"] in ids:
                raise GateError("duplicate run in Actions pagination")
            ids.add(run["id"])
            runs.append(run)
        if len(runs) == total:
            return [run for run in runs if run["head_branch"] == "main"
                    and run["event"] in {"push", "workflow_dispatch"}]
        page += 1


def gate(root, api, *, max_commits=50, timeout=175 * 60, sleep=time.sleep, clock=time.monotonic):
    deadline = clock() + timeout

    def check_time():
        if clock() >= deadline:
            raise GateError("timed out waiting for successful CI on main")

    if git(root, "rev-parse", "--is-shallow-repository").strip() != "false":
        raise GateError("release gate requires a full-history checkout (fetch-depth: 0)")
    tagged = git(root, "rev-parse", "--verify", "HEAD^{commit}").strip()
    main = git(root, "rev-parse", "--verify", "refs/remotes/origin/main^{commit}").strip()
    ancestor = subprocess.run(["git", "merge-base", "--is-ancestor", tagged, main],
                              cwd=root, capture_output=True)
    if ancestor.returncode == 1:
        raise GateError(f"tagged commit {tagged} is not an ancestor of origin/main ({main})")
    if ancestor.returncode:
        raise GateError(f"cannot check origin/main ancestry: {os.fsdecode(ancestor.stderr).strip()}")
    patterns = ignored_patterns(git(root, "show", f"{tagged}:.github/workflows/ci.yml"))
    commits = git(root, "rev-list", "--first-parent", f"--max-count={max_commits}", tagged).splitlines()
    for sha in commits:
        runs = runs_for_commit(api, sha, check_time)
        if runs:
            break
    else:
        raise GateError(f"no CI run on main found within {max_commits} first-parent commits of {tagged}")

    # Disable rename detection so moving a code file under docs still exposes
    # its deleted source path. NUL separators preserve arbitrary file names.
    changed = git(root, "diff", "--no-ext-diff", "--no-textconv", "--no-renames",
                  "--name-only", "-z", sha, tagged, "--").split("\0")
    uncovered = [path for path in changed if path and not any(
        path.startswith(pattern[:-2]) if pattern.endswith("/**") else path == pattern
        for pattern in patterns
    )]
    if uncovered:
        raise GateError("files changed since CI commit " + sha + " are not ignored by CI:\n"
                        + "\n".join(f"  {path!r}" for path in uncovered))

    while True:
        successful = next((run for run in runs if run["status"] == "completed"
                           and run["conclusion"] == "success"), None)
        if successful:
            check_time()
            print(f"Release tag commit {tagged} relies on CI commit {sha}: {successful['html_url']}", flush=True)
            return sha, successful["html_url"]
        if not any(run["status"] != "completed" for run in runs):
            outcomes = ", ".join(f"{run['html_url']} ({run['conclusion']})" for run in runs)
            raise GateError(f"CI on main did not succeed for {sha}: {outcomes or 'runs disappeared'}")
        if deadline - clock() <= 60:
            raise GateError(f"timed out waiting for successful CI on main for {sha}")
        print(f"Waiting 60 seconds for CI on main at {sha}", flush=True)
        sleep(60)
        runs = runs_for_commit(api, sha, check_time)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", required=True)
    parser.add_argument("--timeout-seconds", type=int, default=175 * 60)
    args = parser.parse_args()
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", args.repo):
        parser.error("--repo must be OWNER/REPO")
    if args.timeout_seconds <= 0:
        parser.error("--timeout-seconds must be positive")
    token = os.environ.get("GITHUB_TOKEN")
    if not token:
        parser.error("GITHUB_TOKEN is required")
    try:
        gate(Path.cwd(), GitHub(args.repo, token), timeout=args.timeout_seconds)
    except (GateError, OSError) as error:
        print(f"release CI gate failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
