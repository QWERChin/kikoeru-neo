#!/usr/bin/env python3
"""Audit frontend dependencies with versioned, scoped and expiring exceptions."""

import argparse
from datetime import date, datetime, timezone
import json
from pathlib import Path
import re
import subprocess
import sys


class AuditError(RuntimeError):
    pass


def command_json(web, *args, allowed=(0,)):
    try:
        result = subprocess.run(
            ["pnpm", *args], cwd=web, capture_output=True, text=True, timeout=120
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise AuditError(f"pnpm {args[0]} failed: {error}") from error
    if result.returncode not in allowed:
        detail = result.stderr.strip() or result.stdout.strip()
        raise AuditError(f"pnpm {args[0]} exited {result.returncode}: {detail}")
    try:
        data = json.loads(result.stdout)
    except ValueError as error:
        raise AuditError(f"pnpm {args[0]} returned invalid JSON: {result.stdout}") from error
    if result.returncode == 1 and (not isinstance(data, dict) or not data.get("advisories")):
        raise AuditError(f"pnpm {args[0]} failed without returning advisories: {result.stdout}")
    return data


def load_exceptions(path):
    try:
        entries = json.loads(path.read_text())
    except (OSError, ValueError) as error:
        raise AuditError(f"cannot read {path}: {error}") from error
    if not isinstance(entries, list):
        raise AuditError("audit exceptions must be a JSON array")
    ids = set()
    for entry in entries:
        if not isinstance(entry, dict):
            raise AuditError("each audit exception must be an object")
        for key in ("ghsa", "package", "version", "expires", "reason"):
            if not isinstance(entry.get(key), str) or not entry[key].strip():
                raise AuditError(f"audit exception requires a nonempty {key}")
        ghsa = entry["ghsa"]
        if not re.fullmatch(r"GHSA-[a-z0-9]{4}-[a-z0-9]{4}-[a-z0-9]{4}", ghsa) or ghsa in ids:
            raise AuditError(f"invalid or duplicate advisory ID: {ghsa}")
        ids.add(ghsa)
        try:
            expires = date.fromisoformat(entry["expires"])
        except ValueError as error:
            raise AuditError(f"{ghsa}: expires must be YYYY-MM-DD") from error
        if expires.isoformat() != entry["expires"]:
            raise AuditError(f"{ghsa}: expires must be YYYY-MM-DD")
        via = entry.get("via")
        if (not isinstance(via, dict)
                or any(not isinstance(via.get(key), str) or not via[key] for key in ("package", "version"))):
            raise AuditError(f"{ghsa}: via must name a package and its exact version")
        paths = entry.get("paths")
        if (not isinstance(paths, list) or not paths
                or any(not isinstance(path, str) or not path.startswith(".>")
                       or not path.endswith(f'>{via["package"]}>{entry["package"]}') for path in paths)):
            raise AuditError(f"{ghsa}: paths must end with the reviewed introducer and package")
    return entries


def package_versions(tree):
    if not isinstance(tree, list):
        raise AuditError("pnpm list did not return a dependency tree")
    versions = {}

    def visit(node):
        if not isinstance(node, dict):
            raise AuditError("pnpm list returned an invalid dependency node")
        for section in ("dependencies", "devDependencies", "optionalDependencies"):
            children = node.get(section, {})
            if not isinstance(children, dict):
                raise AuditError("pnpm list returned invalid dependencies")
            for name, child in children.items():
                if not isinstance(child, dict) or not isinstance(child.get("version"), str):
                    raise AuditError(f"pnpm list returned no version for {name}")
                versions.setdefault(name, set()).add(child["version"])
                visit(child)

    for root in tree:
        visit(root)
    return versions


def assess(report, versions, entries, today):
    if not isinstance(report, dict) or report.get("error") or not isinstance(report.get("advisories"), dict):
        raise AuditError("pnpm audit did not return a successful advisory report")
    metadata = report.get("metadata")
    counts = metadata.get("vulnerabilities") if isinstance(metadata, dict) else None
    if (not isinstance(counts, dict) or not counts
            or any(type(value) is not int or value < 0 for value in counts.values())
            or sum(counts.values()) != len(report["advisories"])):
        raise AuditError("pnpm audit report is incomplete or filtered; remove pnpm audit ignore settings")
    exceptions = {entry["ghsa"]: entry for entry in entries}
    seen = set()
    accepted = set()
    errors = []
    for advisory in report["advisories"].values():
        if not isinstance(advisory, dict):
            raise AuditError("pnpm audit returned an invalid advisory")
        url = advisory.get("url")
        match = re.fullmatch(
            r"https://github.com/advisories/(GHSA-[a-z0-9]{4}-[a-z0-9]{4}-[a-z0-9]{4})",
            url,
        ) if isinstance(url, str) else None
        if not match:
            raise AuditError("pnpm audit returned an advisory without a GitHub advisory URL")
        ghsa = match[1]
        seen.add(ghsa)
        entry = exceptions.get(ghsa)
        if entry is None:
            errors.append(f'{ghsa}: {advisory.get("module_name", "unknown package")} has no reviewed exception')
            continue
        findings = advisory.get("findings")
        if not isinstance(findings, list) or not findings:
            raise AuditError(f"{ghsa}: audit report has no affected versions and paths")
        problems = []
        if advisory.get("module_name") != entry["package"]:
            problems.append("affected package changed")
        for finding in findings:
            if not isinstance(finding, dict):
                raise AuditError(f"{ghsa}: invalid finding")
            paths = finding.get("paths")
            if not isinstance(paths, list) or not paths or any(not isinstance(path, str) for path in paths):
                raise AuditError(f"{ghsa}: audit report has no dependency paths")
            if finding.get("version") != entry["version"]:
                problems.append(f'affected version changed from {entry["version"]}')
            for path in set(paths) - set(entry["paths"]):
                problems.append(f"unreviewed dependency path: {path}")
        via = entry["via"]
        actual_package = versions.get(entry["package"], set())
        if actual_package != {entry["version"]}:
            problems.append(f'locked versions of {entry["package"]} changed to {sorted(actual_package)}')
        actual = versions.get(via["package"], set())
        if actual != {via["version"]}:
            problems.append(f'introducer {via["package"]} changed from {via["version"]} to {sorted(actual)}')
        if date.fromisoformat(entry["expires"]) <= today:
            problems.append(f'exception expired on {entry["expires"]} UTC')
        if problems:
            errors.extend(f"{ghsa}: {problem}; reassess the exception" for problem in problems)
        else:
            accepted.add(ghsa)
    return accepted, set(exceptions) - seen, errors


def check(web, exceptions_path, *, prune=False, today=None):
    entries = load_exceptions(exceptions_path)
    report = command_json(web, "audit", "--json", "--audit-level", "low", allowed=(0, 1))
    tree = command_json(web, "list", "--lockfile-only", "--json", "--depth", "Infinity")
    accepted, stale, errors = assess(
        report, package_versions(tree), entries, today or datetime.now(timezone.utc).date()
    )
    if stale and not prune:
        errors.append(
            f"obsolete exceptions: {', '.join(sorted(stale))}; "
            "run python3 scripts/pnpm-audit.py --prune and commit the cleanup"
        )
    if errors:
        raise AuditError("\n".join(errors))
    # A failed or incomplete audit must never remove an exception.
    if stale:
        entries = [entry for entry in entries if entry["ghsa"] not in stale]
        exceptions_path.write_text(json.dumps(entries, indent=2) + "\n")
        print(f"Removed obsolete exceptions: {', '.join(sorted(stale))}")
    for entry in entries:
        if entry["ghsa"] in accepted:
            print(f'Reviewed exception: {entry["ghsa"]}, {entry["package"]}@{entry["version"]}, '
                  f'expires {entry["expires"]} UTC')
    print(f"Frontend audit passed with {len(accepted)} reviewed exception(s)")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prune", action="store_true", help="remove obsolete exceptions after a successful audit")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    try:
        check(root / "web", root / "scripts/pnpm-audit-exceptions.json", prune=args.prune)
    except AuditError as error:
        print(f"Frontend audit failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
