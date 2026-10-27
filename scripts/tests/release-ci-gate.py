"""Test the release gate with synthetic Git history and no network access."""

import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import urllib.error
import urllib.parse

spec = importlib.util.spec_from_file_location("gate", Path(__file__).parents[1] / "release-ci-gate.py")
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)

WORKFLOW = '''name: CI
on:
  push:
    branches: [main]
    paths-ignore: ["book/**", "docs/**", "README.md"]
  pull_request:
    paths-ignore: ["different/**"]
jobs:
  check:
    runs-on: ubuntu-latest
'''


def run(sha, id=1, **changes):
    return dict({"id": id, "head_sha": sha, "head_branch": "main", "event": "push",
                 "path": ".github/workflows/ci.yml", "status": "completed",
                 "conclusion": "success", "html_url": f"https://github.com/owner/repo/actions/runs/{id}"},
                **changes)


class FakeGitHub:
    def __init__(self, snapshots=None):
        self.snapshots = snapshots or {}
        self.polls = {}
        self.calls = []

    def request(self, path):
        self.calls.append(path)
        url = urllib.parse.urlparse(path)
        assert url.path == "/actions/workflows/ci.yml/runs"
        query = urllib.parse.parse_qs(url.query)
        assert query["per_page"] == ["100"]
        sha, page = query["head_sha"][0], int(query["page"][0])
        if page == 1:
            self.polls[sha] = self.polls.get(sha, -1) + 1
        snapshots = self.snapshots.get(sha, [[]])
        runs = snapshots[min(self.polls[sha], len(snapshots) - 1)]
        return {"total_count": len(runs), "workflow_runs": runs[(page - 1) * 100:page * 100]}


class Tests(unittest.TestCase):
    def setUp(self):
        # Ignore the developer's signing, hooks and Git identity configuration.
        env = patch.dict(os.environ, {"GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull})
        env.start()
        self.addCleanup(env.stop)
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.git("init", "--initial-branch=main")
        self.git("config", "user.name", "Synthetic Test")
        self.git("config", "user.email", "synthetic@example.invalid")
        self.initial = self.commit({".github/workflows/ci.yml": WORKFLOW, "src/main.rs": "fn main() {}\n"})
        self.sleeps = []
        self.now = 0
        self.output = io.StringIO()

    def git(self, *args):
        return subprocess.run(["git", *args], cwd=self.root, check=True,
                              capture_output=True, text=True).stdout.strip()

    def commit(self, files, *, on_main=True):
        for name, content in files.items():
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            if content is None:
                path.unlink()
            else:
                path.write_text(content)
        self.git("add", ".")
        self.git("commit", "-m", "Synthetic change")
        sha = self.git("rev-parse", "HEAD")
        if on_main:
            self.git("update-ref", "refs/remotes/origin/main", sha)
        return sha

    def sleep(self, seconds):
        self.sleeps.append(seconds)
        self.now += seconds

    def check(self, api, **options):
        with contextlib.redirect_stdout(self.output):
            return gate.gate(self.root, api, sleep=self.sleep, clock=lambda: self.now, **options)

    def test_tag_on_passed_commit(self):
        self.git("checkout", "--detach", self.initial)
        api = FakeGitHub({self.initial: [[run(self.initial)]]})
        self.assertEqual(self.check(api), (self.initial, run(self.initial)["html_url"]))
        self.assertIn(self.initial, self.output.getvalue())
        self.assertIn(run(self.initial)["html_url"], self.output.getvalue())
        self.assertEqual(self.sleeps, [])

    def test_docs_only_tag_after_passed_commit(self):
        tagged = self.commit({"book/deep/page.md": "Text", "docs/.hidden": "Text", "README.md": "Text"})
        self.commit({"src/later.rs": "Code after the tag"})
        self.git("checkout", "--detach", tagged)
        api = FakeGitHub({self.initial: [[run(self.initial, event="workflow_dispatch")]]})
        self.assertEqual(self.check(api)[0], self.initial)
        self.assertEqual(list(api.polls), [tagged, self.initial])

    def test_manual_retry_of_tag_predating_gate_script(self):
        source = Path(spec.origin).read_text()
        workflow_sha = self.commit({"scripts/release-ci-gate.py": source})
        self.git("checkout", "--detach", self.initial)
        self.assertFalse((self.root / "scripts/release-ci-gate.py").exists())
        with tempfile.TemporaryDirectory() as runner_temp:
            script = Path(runner_temp) / "release-ci-gate.py"
            script.write_text(self.git("show", f"{workflow_sha}:scripts/release-ci-gate.py"))
            retry_spec = importlib.util.spec_from_file_location("retry", script)
            retry = importlib.util.module_from_spec(retry_spec)
            retry_spec.loader.exec_module(retry)
            api = FakeGitHub({self.initial: [[run(self.initial)]]})
            with patch.dict(os.environ, {"GITHUB_SHA": workflow_sha}), contextlib.redirect_stdout(self.output):
                self.assertEqual(retry.gate(self.root, api)[0], self.initial)

    def test_code_after_last_ci_fails_and_names_files(self):
        self.commit({"src/main.rs": "changed", "README.md.bak": "not ignored", "books/page.md": "not ignored"})
        api = FakeGitHub({self.initial: [[run(self.initial)]]})
        with self.assertRaises(gate.GateError) as error:
            self.check(api)
        for name in ["src/main.rs", "README.md.bak", "books/page.md"]:
            self.assertIn(name, str(error.exception))

    def test_rename_from_code_to_docs_fails(self):
        self.commit({"src/main.rs": None, "docs/main.rs": "fn main() {}\n"})
        with self.assertRaisesRegex(gate.GateError, "src/main.rs"):
            self.check(FakeGitHub({self.initial: [[run(self.initial)]]}))

    def test_failed_nearest_run_does_not_fall_back(self):
        tagged = self.commit({"book/page.md": "Text"})
        for conclusion in ["failure", "cancelled", "skipped", "neutral", "timed_out", "action_required"]:
            with self.subTest(conclusion=conclusion):
                api = FakeGitHub({tagged: [[run(tagged, conclusion=conclusion)]],
                                  self.initial: [[run(self.initial)]]})
                with self.assertRaisesRegex(gate.GateError, "did not succeed"):
                    self.check(api)
                self.assertNotIn(self.initial, api.polls)

    def test_pending_run_then_success(self):
        api = FakeGitHub({self.initial: [
            [run(self.initial, conclusion="failure"), run(self.initial, 2, status="in_progress", conclusion=None)],
            [run(self.initial, conclusion="failure"), run(self.initial, 2)],
        ]})
        self.assertEqual(self.check(api)[1], run(self.initial, 2)["html_url"])
        self.assertEqual(self.sleeps, [60])

    def test_pending_run_times_out(self):
        api = FakeGitHub({self.initial: [[run(self.initial, status="queued", conclusion=None)]]})
        with self.assertRaisesRegex(gate.GateError, "timed out"):
            self.check(api, timeout=121)
        self.assertEqual(self.sleeps, [60, 60])

    def test_pending_run_disappears(self):
        api = FakeGitHub({self.initial: [[run(self.initial, status="waiting", conclusion=None)], []]})
        with self.assertRaisesRegex(gate.GateError, "runs disappeared"):
            self.check(api)

    def test_success_is_enough_among_multiple_runs(self):
        api = FakeGitHub({self.initial: [[run(self.initial, conclusion="failure"),
                                        run(self.initial, 2, status="queued", conclusion=None),
                                        run(self.initial, 3)]]})
        self.assertEqual(self.check(api)[1], run(self.initial, 3)["html_url"])
        self.assertEqual(self.sleeps, [])

    def test_other_events_and_branches_are_not_evidence(self):
        for changes in [{"event": "pull_request"}, {"event": "schedule"},
                        {"head_branch": "topic"}, {"head_branch": None}]:
            with self.subTest(changes=changes):
                with self.assertRaisesRegex(gate.GateError, "no CI run on main"):
                    self.check(FakeGitHub({self.initial: [[run(self.initial, **changes)]]}))

    def test_tag_not_on_main_fails_before_api(self):
        self.git("checkout", "--detach", self.initial)
        tagged = self.commit({"src/topic.rs": "not on main"}, on_main=False)
        api = FakeGitHub({tagged: [[run(tagged)]]})
        with self.assertRaisesRegex(gate.GateError, "not an ancestor of origin/main"):
            self.check(api)
        self.assertEqual(api.calls, [])

    def test_missing_main_fails_before_api(self):
        self.git("update-ref", "-d", "refs/remotes/origin/main")
        api = FakeGitHub({self.initial: [[run(self.initial)]]})
        with self.assertRaisesRegex(gate.GateError, "origin/main.*failed"):
            self.check(api)
        self.assertEqual(api.calls, [])

    def test_shallow_history_fails_before_api(self):
        with tempfile.TemporaryDirectory() as directory:
            clone = Path(directory) / "shallow"
            self.git("clone", "--depth=1", self.root.as_uri(), str(clone))
            api = FakeGitHub({self.initial: [[run(self.initial)]]})
            with self.assertRaisesRegex(gate.GateError, "full-history checkout"):
                gate.gate(clone, api)
            self.assertEqual(api.calls, [])

    def test_first_parent_only(self):
        self.git("checkout", "--detach", self.initial)
        side = self.commit({"book/side.md": "side"}, on_main=False)
        self.git("checkout", "main")
        main = self.commit({"book/main.md": "main"})
        self.git("merge", "--no-ff", "-m", "Synthetic merge", side)
        self.git("update-ref", "refs/remotes/origin/main", "HEAD")
        api = FakeGitHub({side: [[run(side)]], main: [[run(main)]]})
        self.assertEqual(self.check(api)[0], main)
        self.assertNotIn(side, api.polls)

    def test_history_search_is_bounded(self):
        tagged = self.commit({"book/page.md": "Text"})
        api = FakeGitHub({self.initial: [[run(self.initial)]]})
        with self.assertRaisesRegex(gate.GateError, "within 1 first-parent commits"):
            self.check(api, max_commits=1)
        self.assertEqual(list(api.polls), [tagged])

    def test_unknown_paths_ignore_form_fails_even_on_success(self):
        original = 'paths-ignore: ["book/**", "docs/**", "README.md"]'
        for replacement in ["paths-ignore:\n      - book/**", "paths-ignore: *ignored",
                            "paths-ignore: ['book/**']", "paths-ignore: [true]",
                            'paths-ignore: ["book/**"]\n    paths-ignore: ["docs/**"]']:
            with self.subTest(replacement=replacement):
                sha = self.commit({".github/workflows/ci.yml": WORKFLOW.replace(original, replacement)})
                with self.assertRaisesRegex(gate.GateError, "unsupported paths-ignore form"):
                    self.check(FakeGitHub({sha: [[run(sha)]]}))

    def test_unknown_globs_fail(self):
        for pattern in ["*.md", "docs/*", "**", "!src/**", "docs/?.md", "docs/[ab]/**",
                        "docs/{a,b}/**", "docs/../src/**", "/docs/**", "docs/**/page.md", "docs/a+.md"]:
            with self.subTest(pattern=pattern):
                workflow = WORKFLOW.replace('["book/**", "docs/**", "README.md"]', json.dumps([pattern]))
                with self.assertRaisesRegex(gate.GateError, "unsupported paths-ignore glob"):
                    gate.ignored_patterns(workflow)

    def test_paths_ignore_is_read_from_tag_not_working_tree_or_pr(self):
        (self.root / ".github/workflows/ci.yml").write_text("not the tagged workflow")
        self.assertEqual(gate.ignored_patterns(WORKFLOW), ["book/**", "docs/**", "README.md"])
        self.check(FakeGitHub({self.initial: [[run(self.initial)]]}))

    def test_success_on_later_page(self):
        runs = [run(self.initial, i, conclusion="failure") for i in range(1, 101)]
        api = FakeGitHub({self.initial: [runs + [run(self.initial, 101)]]})
        self.assertEqual(self.check(api)[1], run(self.initial, 101)["html_url"])
        self.assertEqual(len(api.calls), 2)

    def test_bad_pagination_fails_even_after_success(self):
        valid = [run(self.initial, i) for i in range(1, 101)]
        cases = [
            [{"total_count": 1000, "workflow_runs": valid}],
            [{"total_count": 2, "workflow_runs": valid[:1]}],
            [{"total_count": 0, "workflow_runs": valid[:1]}],
            [{"total_count": 101, "workflow_runs": valid}, {"total_count": 101, "workflow_runs": []}],
            [{"total_count": 101, "workflow_runs": valid}, {"total_count": 100, "workflow_runs": []}],
            [{"total_count": 101, "workflow_runs": valid}, {"total_count": 101, "workflow_runs": valid[:1]}],
        ]
        for responses in cases:
            with self.subTest(responses=len(responses)):
                api = FakeGitHub()
                api.request = lambda path, responses=iter(responses): next(responses)
                with self.assertRaisesRegex(gate.GateError, "pagination"):
                    self.check(api)

    def test_unexpected_json_fails(self):
        valid = run(self.initial)
        responses = [None, [], {}, {"total_count": True, "workflow_runs": []},
                     {"total_count": -1, "workflow_runs": []}, {"total_count": 0, "workflow_runs": {}}]
        bad_runs = [None, {}, dict(valid, head_sha="0" * 40), dict(valid, status="unknown"),
                    dict(valid, conclusion=None), dict(valid, status="queued"),
                    dict(valid, path=".github/workflows/other.yml"), dict(valid, id=True)]
        responses += [{"total_count": 2, "workflow_runs": [valid, bad]} for bad in bad_runs]
        for response in responses:
            with self.subTest(response=response):
                api = FakeGitHub()
                api.request = lambda path: response
                with self.assertRaisesRegex(gate.GateError, "unexpected Actions JSON"):
                    self.check(api)

    def test_api_errors_fail_including_after_first_page_success(self):
        api = FakeGitHub()
        with patch.object(api, "request", side_effect=gate.GateError("API unavailable")):
            with self.assertRaisesRegex(gate.GateError, "API unavailable"):
                self.check(api)
        first = {"total_count": 101, "workflow_runs": [run(self.initial, i) for i in range(1, 101)]}
        with patch.object(api, "request", side_effect=[first, gate.GateError("API unavailable")]):
            with self.assertRaisesRegex(gate.GateError, "API unavailable"):
                self.check(api)

    def test_http_errors_and_invalid_json_are_not_empty_results(self):
        api = gate.GitHub("owner/repo", "synthetic-token")
        for status in [401, 403, 404, 429, 500]:
            with self.subTest(status=status):
                error = urllib.error.HTTPError(api.base, status, "failure", {}, io.BytesIO())
                self.addCleanup(error.close)
                with patch.object(api.opener, "open", side_effect=error):
                    with self.assertRaisesRegex(gate.GateError, "Actions API request failed"):
                        api.request("/actions/workflows/ci.yml/runs")
        with patch.object(api.opener, "open") as opener:
            response = opener.return_value.__enter__.return_value
            response.status = 200
            response.read.return_value = b"not JSON"
            with self.assertRaisesRegex(gate.GateError, "Actions API request failed"):
                api.request("/actions/workflows/ci.yml/runs")

    def test_redirect_is_an_error(self):
        with self.assertRaisesRegex(gate.GateError, "redirect"):
            gate.NoRedirect().redirect_request(None, None, 302, "Found", {}, "https://elsewhere.invalid")


unittest.main()
