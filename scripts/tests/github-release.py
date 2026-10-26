"""Test retries without credentials, network access or a real release."""

import importlib.util
import io
from pathlib import Path
import unittest
import urllib.error
import urllib.request

spec = importlib.util.spec_from_file_location("release", Path(__file__).parents[1] / "github-release.py")
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class FakeGitHub:
    def __init__(self, assets=None, exists=True, draft=False):
        self.assets = dict(assets or {})
        self.exists = exists
        self.release = {"id": 1, "upload_url": "https://uploads.invalid/assets{?name}", "draft": draft}
        self.writes = []
        self.fail_upload = None

    def request(self, url, *, method="GET", data=None, accept=None):
        if method != "GET":
            self.writes.append((method, url))
            if url == "/releases":
                self.exists = True
                self.release["draft"] = True
                return self.release
            if method == "PATCH":
                self.release["draft"] = False
                return self.release
            name = url.split("?name=")[1]
            if name == self.fail_upload:
                raise RuntimeError("interrupted upload")
            self.assets[name] = data
            return {}
        if url.startswith("/git/ref/"):
            return {"ref": "refs/tags/v0.1.0"}
        if url.startswith("/releases/tags/"):
            return self.release if self.exists else None
        if url.startswith("/releases/1/assets?"):
            return [{"name": name, "url": "asset:" + name} for name in self.assets]
        return self.assets[url.removeprefix("asset:")]


class Tests(unittest.TestCase):
    assets = {"binary": b"elf", "SHA256SUMS": b"hash"}

    def test_new_release_is_published_after_uploads(self):
        api = FakeGitHub(exists=False)
        release.publish(api, "v0.1.0", self.assets)
        self.assertEqual(api.assets, self.assets)
        self.assertEqual(api.writes[-1], ("PATCH", "/releases/1"))

    def test_identical_retry_does_not_write(self):
        api = FakeGitHub(self.assets)
        release.publish(api, "v0.1.0", self.assets)
        self.assertEqual(api.writes, [])

    def test_conflict_fails_before_any_upload(self):
        api = FakeGitHub({"SHA256SUMS": b"different"})
        with self.assertRaisesRegex(RuntimeError, "another checksum"):
            release.publish(api, "v0.1.0", self.assets)
        self.assertEqual(api.writes, [])

    def test_partial_draft_is_resumed(self):
        api = FakeGitHub(exists=False)
        api.fail_upload = "SHA256SUMS"
        with self.assertRaisesRegex(RuntimeError, "interrupted"):
            release.publish(api, "v0.1.0", self.assets)
        self.assertTrue(api.release["draft"])
        api.fail_upload = None
        api.writes.clear()
        release.publish(api, "v0.1.0", self.assets)
        self.assertEqual(len(api.writes), 2)
        self.assertFalse(api.release["draft"])

    def test_missing_release_on_auth_failure_is_not_assumed(self):
        api = FakeGitHub()
        def denied(*args, **kwargs):
            raise PermissionError("403")
        api.request = denied
        with self.assertRaises(PermissionError):
            release.publish(api, "v0.1.0", self.assets)
        self.assertEqual(api.writes, [])

    def test_missing_tag_does_not_create_release(self):
        api = FakeGitHub(exists=False)
        api.request = lambda *args, **kwargs: None
        with self.assertRaisesRegex(RuntimeError, "tag .* does not exist"):
            release.publish(api, "v0.1.0", self.assets)
        self.assertEqual(api.writes, [])

    def test_asset_redirect_does_not_forward_token(self):
        request = urllib.request.Request(
            "https://api.github.com/repos/owner/repo/releases/assets/1",
            headers={"Authorization": "Bearer synthetic-token"},
        )
        redirected = release.AssetRedirect().redirect_request(
            request, None, 302, "Found", {}, "https://release-assets.githubusercontent.com/asset",
        )
        self.assertFalse(redirected.has_header("Authorization"))

    def test_only_read_404_means_missing(self):
        api = release.GitHub("owner/repo", "synthetic-token")
        class FailingOpener:
            status = 404
            def open(self, request, **kwargs):
                error = urllib.error.HTTPError(request.full_url, self.status, "failure", {}, io.BytesIO())
                error.close()
                raise error
        api.opener = FailingOpener()
        self.assertIsNone(api.request("/releases/tags/v0.1.0"))
        with self.assertRaises(urllib.error.HTTPError):
            api.request("/releases", method="POST", data={})
        for status in [401, 403, 500]:
            api.opener.status = status
            with self.assertRaises(urllib.error.HTTPError):
                api.request("/releases/tags/v0.1.0")


unittest.main()
