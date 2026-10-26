#!/usr/bin/env python3
"""Publish immutable standalone assets, including safe retries of partial releases."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import tomllib
import urllib.error
import urllib.parse
import urllib.request


class AssetRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, url):
        redirected = super().redirect_request(request, fp, code, message, headers, url)
        if urllib.parse.urlparse(request.full_url).netloc != urllib.parse.urlparse(url).netloc:
            redirected.remove_header("Authorization")
        return redirected


class GitHub:
    def __init__(self, repo, token):
        self.base = f"https://api.github.com/repos/{repo}"
        self.token = token
        self.opener = urllib.request.build_opener(AssetRedirect())

    def request(self, url, *, method="GET", data=None, accept="application/vnd.github+json"):
        if url.startswith("/"):
            url = self.base + url
        headers = {"Authorization": f"Bearer {self.token}", "Accept": accept,
                   "X-GitHub-Api-Version": "2022-11-28"}
        if isinstance(data, dict):
            headers["Content-Type"] = "application/json"
            data = json.dumps(data).encode()
        elif data is not None:
            headers["Content-Type"] = "application/octet-stream"
        request = urllib.request.Request(url, data=data, method=method, headers=headers)
        try:
            with self.opener.open(request, timeout=120) as response:
                body = response.read()
                return body if accept == "application/octet-stream" else json.loads(body)
        except urllib.error.HTTPError as error:
            if error.code == 404 and method == "GET":
                error.close()
                return None
            raise


def publish(api, tag, assets):
    # GitHub can create a tag as a side effect of creating a release.
    # Require the existing tag even when called outside the workflow.
    if api.request(f"/git/ref/tags/{tag}") is None:
        raise RuntimeError(f"tag {tag} does not exist")
    release = api.request(f"/releases/tags/{tag}")
    existing = {}
    if release is not None:
        page = 1
        while True:
            batch = api.request(f"/releases/{release['id']}/assets?per_page=100&page={page}")
            for asset in batch:
                if asset["name"] in existing:
                    raise RuntimeError(f"duplicate asset {asset['name']}")
                existing[asset["name"]] = asset
            if len(batch) < 100:
                break
            page += 1
    # Compare the downloaded bytes, not an optional API digest. Check all
    # existing assets before uploading any missing ones.
    for name, data in assets.items():
        if name in existing:
            remote = api.request(existing[name]["url"], accept="application/octet-stream")
            if remote is None or hashlib.sha256(remote).digest() != hashlib.sha256(data).digest():
                raise RuntimeError(f"{name} already exists with another checksum; refusing replacement")
            print(f"keeping identical asset {name}")
    if release is None:
        release = api.request("/releases", method="POST", data={
            "tag_name": tag, "name": tag, "draft": True,
            "body": "The Linux musl server embeds the web UI. Install ffmpeg and ffprobe separately.",
        })
    upload = release["upload_url"].split("{", 1)[0]
    for name, data in assets.items():
        if name not in existing:
            api.request(f"{upload}?name={urllib.parse.quote(name)}", method="POST", data=data)
            print(f"uploaded {name}")
    if release["draft"]:
        api.request(f"/releases/{release['id']}", method="PATCH", data={"draft": False})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--cargo", type=Path, default=Path("Cargo.toml"))
    args = parser.parse_args()
    version = tomllib.loads(args.cargo.read_text())["workspace"]["package"]["version"]
    if not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", version):
        parser.error("only X.Y.Z releases are supported")
    if args.tag != f"v{version}":
        parser.error(f"tag must match Cargo.toml version v{version}")
    name = f"kikoeru-neo-{version}-x86_64-linux-musl"
    binary = args.binary.read_bytes()
    sums = f"{hashlib.sha256(binary).hexdigest()}  {name}\n".encode()
    publish(GitHub(args.repo, os.environ["GITHUB_TOKEN"]), args.tag, {name: binary, "SHA256SUMS": sums})


if __name__ == "__main__":
    main()
