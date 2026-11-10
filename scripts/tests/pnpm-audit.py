"""Exercise exception scope and cleanup with synthetic pnpm responses."""

import contextlib
import copy
from datetime import date
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location("audit", Path(__file__).parents[1] / "pnpm-audit.py")
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)

ENTRY = {
    "ghsa": "GHSA-aaaa-bbbb-cccc",
    "package": "synthetic-merge",
    "version": "1.0.0",
    "via": {"package": "synthetic-plugin", "version": "2.0.0"},
    "paths": [".>synthetic-plugin>synthetic-merge"],
    "expires": "2026-12-10",
    "reason": "Only trusted build configuration reaches this synthetic merger.",
}


def report():
    return {
        "advisories": {
            "123": {
                "module_name": "synthetic-merge",
                "url": "https://github.com/advisories/GHSA-aaaa-bbbb-cccc",
                "findings": [{"version": "1.0.0", "paths": ENTRY["paths"].copy()}],
            }
        },
        "metadata": {"vulnerabilities": {"low": 1}},
    }


def tree():
    return [{"devDependencies": {
        "synthetic-plugin": {"version": "2.0.0", "dependencies": {
            "synthetic-merge": {"version": "1.0.0"}
        }}
    }}]


class Tests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.web = self.root / "web"
        self.web.mkdir()
        self.policy = self.root / "exceptions.json"
        self.policy.write_text(json.dumps([ENTRY], indent=2) + "\n")
        self.original = self.policy.read_bytes()
        self.report = report()
        self.tree = tree()
        self.today = date(2026, 11, 10)
        self.calls = []
        self.output = io.StringIO()
        mock = patch.object(audit.subprocess, "run", side_effect=self.pnpm)
        mock.start()
        self.addCleanup(mock.stop)

    def pnpm(self, args, **kwargs):
        self.assertEqual(args[0], "pnpm")
        self.assertEqual(kwargs["cwd"], self.web)
        self.calls.append(args)
        if args[1] == "audit":
            self.assertEqual(args[2:], ["--json", "--audit-level", "low"])
            return subprocess.CompletedProcess(args, bool(self.report["advisories"]), json.dumps(self.report), "")
        self.assertEqual(args[1:], ["list", "--lockfile-only", "--json", "--depth", "Infinity"])
        return subprocess.CompletedProcess(args, 0, json.dumps(self.tree), "")

    def check(self, *, prune=False):
        with contextlib.redirect_stdout(self.output):
            audit.check(self.web, self.policy, prune=prune, today=self.today)

    def assert_rejected(self, message, *, prune=False):
        with self.assertRaisesRegex(audit.AuditError, message):
            self.check(prune=prune)
        self.assertEqual(self.policy.read_bytes(), self.original)

    def test_reviewed_build_dependency_passes_without_modifying_policy(self):
        self.check()
        self.assertEqual(self.policy.read_bytes(), self.original)
        self.assertIn("expires 2026-12-10 UTC", self.output.getvalue())
        self.assertEqual(len(self.calls), 2)

    def test_direct_runtime_dependency_is_not_covered_by_the_build_exception(self):
        self.report["advisories"]["123"]["findings"][0]["paths"].append(".>synthetic-merge")
        self.assert_rejected("unreviewed dependency path")

    def test_new_advisory_for_the_same_package_is_not_ignored(self):
        self.report["advisories"]["456"] = copy.deepcopy(self.report["advisories"]["123"])
        self.report["advisories"]["456"]["url"] = "https://github.com/advisories/GHSA-dddd-eeee-ffff"
        self.report["metadata"]["vulnerabilities"]["low"] = 2
        self.assert_rejected("GHSA-dddd-eeee-ffff.*no reviewed exception")

    def test_changed_affected_version_requires_reassessment(self):
        self.report["advisories"]["123"]["findings"][0]["version"] = "1.0.1"
        self.assert_rejected("affected version changed")

    def test_changed_introducer_requires_reassessment(self):
        self.tree[0]["devDependencies"]["synthetic-plugin"]["version"] = "2.0.1"
        self.assert_rejected("introducer synthetic-plugin changed")

    def test_lockfile_version_must_also_match_the_exception(self):
        self.tree[0]["devDependencies"]["synthetic-plugin"]["dependencies"]["synthetic-merge"]["version"] = "1.0.1"
        self.assert_rejected("locked versions of synthetic-merge changed")

    def test_expiration_applies_on_the_due_date_and_after_it(self):
        for day in (date(2026, 12, 10), date(2026, 12, 11)):
            with self.subTest(day=day):
                self.today = day
                self.assert_rejected("exception expired", prune=True)

    def fix_dependency(self):
        self.report = {"advisories": {}, "metadata": {"vulnerabilities": {"low": 0}}}
        self.tree[0]["devDependencies"]["synthetic-plugin"]["dependencies"]["synthetic-merge"]["version"] = "1.0.1"

    def test_fixed_dependency_makes_the_old_exception_fail_ci(self):
        self.fix_dependency()
        self.assert_rejected("obsolete exceptions.*--prune")

    def test_prune_removes_only_obsolete_exceptions(self):
        entries = [copy.deepcopy(ENTRY), copy.deepcopy(ENTRY)]
        entries[1]["ghsa"] = "GHSA-dddd-eeee-ffff"
        self.policy.write_text(json.dumps(entries))
        self.check(prune=True)
        self.assertEqual(json.loads(self.policy.read_text()), [ENTRY])
        self.assertIn("Removed obsolete exceptions: GHSA-dddd-eeee-ffff", self.output.getvalue())

    def test_prune_can_remove_a_fixed_exception_even_after_its_due_date(self):
        self.fix_dependency()
        self.today = date(2027, 1, 1)
        self.check(prune=True)
        self.assertEqual(json.loads(self.policy.read_text()), [])

    def test_new_vulnerability_prevents_pruning(self):
        self.report["advisories"]["123"]["url"] = "https://github.com/advisories/GHSA-dddd-eeee-ffff"
        self.assert_rejected("no reviewed exception", prune=True)

    def test_filtered_or_incomplete_report_cannot_remove_exceptions(self):
        for broken in (
            {"advisories": {}, "metadata": {"vulnerabilities": {"low": 1}}},
            {"advisories": {}, "error": "registry unavailable"},
            {"advisories": {}},
            {"advisories": {}, "metadata": None},
        ):
            with self.subTest(report=broken):
                self.report = broken
                self.assert_rejected("report", prune=True)

    def test_missing_finding_paths_cannot_be_treated_as_reviewed(self):
        self.report["advisories"]["123"]["findings"][0]["paths"] = []
        self.assert_rejected("no dependency paths", prune=True)

    def test_failed_pnpm_command_never_prunes(self):
        for code, output in ((2, "registry unavailable"), (1, '{"advisories":{}}'), (0, "not JSON")):
            with self.subTest(code=code, output=output):
                with patch.object(audit.subprocess, "run", return_value=subprocess.CompletedProcess(
                    ["pnpm", "audit"], code, output, "transport failed"
                )):
                    self.assert_rejected("pnpm audit", prune=True)

    def test_timeout_never_prunes(self):
        with patch.object(audit.subprocess, "run", side_effect=subprocess.TimeoutExpired("pnpm audit", 120)):
            self.assert_rejected("pnpm audit failed", prune=True)

    def test_invalid_exception_metadata_is_rejected_before_audit(self):
        for field, value in (("reason", ""), ("expires", "never"), ("paths", []), ("via", {})):
            with self.subTest(field=field):
                entry = copy.deepcopy(ENTRY)
                entry[field] = value
                self.policy.write_text(json.dumps([entry]))
                with self.assertRaises(audit.AuditError):
                    self.check()
        self.assertEqual(self.calls, [])

    def test_committed_exception_metadata_is_valid(self):
        path = Path(__file__).parents[1] / "pnpm-audit-exceptions.json"
        audit.load_exceptions(path)


if __name__ == "__main__":
    unittest.main()
