#!/usr/bin/env python3
"""Which audio formats a library holds and how kikoeru-neo plays them.

Samples tracks per file extension from a running server, asks `prepare`
how each would play for a given browser, and groups the answers by
container and codec. The decision comes from the server itself
(neo-media's decide.rs), so the survey cannot drift from the real rules.
Transcodes that `prepare` starts are cancelled at once. The report holds
extensions, formats, codecs and counts only, never titles or paths.
book/src/testing.md "音频格式调查" explains how to run it.
"""

import argparse
import collections
import getpass
import json
import random
import sqlite3
import sys
import time
import urllib.error
import urllib.request

# CANDIDATE_TYPES in web/src/lib/player/prepare.ts, which mirrors
# decide.rs, without ALAC, which Chrome does not play. This is an assumption
# about Android Chrome; pass the device's real canPlayType answers with
# --can-play when they are known.
ANDROID_CHROME = [
    "audio/mpeg",
    "audio/mp4",
    'audio/mp4; codecs="mp4a.40.2"',
    "audio/aac",
    "audio/wav",
    "audio/flac",
    "audio/ogg",
    'audio/ogg; codecs="vorbis"',
    'audio/ogg; codecs="opus"',
    "audio/opus",
    "audio/webm",
    'audio/webm; codecs="opus"',
    "video/mp4",
    "video/webm",
]


class Api:
    def __init__(self, base):
        self.base = base.rstrip("/")
        self.token = None

    def call(self, method, path, body=None):
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(self.base + path, data=data, method=method)
        req.add_header("content-type", "application/json")
        if self.token:
            req.add_header("authorization", f"Bearer {self.token}")
        try:
            with urllib.request.urlopen(req, timeout=120) as resp:
                payload = resp.read()
                return resp.status, json.loads(payload) if payload else None
        except urllib.error.HTTPError as e:
            payload = e.read()
            try:
                return e.code, json.loads(payload)
            except ValueError:
                return e.code, None


def extension(path):
    name = path.rsplit("/", 1)[-1]
    return name.rsplit(".", 1)[-1].lower() if "." in name else ""


def playable_tracks(api):
    by_ext = collections.defaultdict(list)
    page = 1
    while True:
        status, body = api.call("GET", f"/api/works?page={page}&page_size=100")
        if status != 200:
            sys.exit(f"cannot list works: {status} {body}")
        for work in body["items"]:
            _, detail = api.call("GET", f"/api/works/{work['id']}")
            for track in detail["tracks"]:
                if track["kind"] in ("audio", "video") and not track["missing"]:
                    by_ext[extension(track["path"])].append(track["id"])
        if page * 100 >= body["total"]:
            return by_ext
        page += 1


def decide(api, track_id, can_play, finish_transcode):
    """`finish_transcode` lets one transcode run to its end, because a
    format that must be transcoded can still be refused by ffmpeg's input
    whitelist. Other transcodes are cancelled to spare the provider."""
    status, body = api.call("POST", f"/api/tracks/{track_id}/prepare",
                            {"can_play": can_play, "force_fallback": False})
    if status == 200:
        return "direct" if body["url"].endswith("/stream") else "transcoded already"
    if status == 202 and not finish_transcode:
        cancelled, _ = api.call("POST", f"/api/tasks/{body['task_id']}/cancel")
        # 400 means the task already ended, which is as good as cancelled.
        if cancelled not in (204, 400):
            # Going on would leave every later transcode running too.
            sys.exit(f"cannot cancel task {body['task_id']}: {cancelled}")
        return "transcode"
    if status == 202:
        while True:
            _, task = api.call("GET", f"/api/tasks/{body['task_id']}")
            if task["state"] not in ("queued", "running"):
                return "transcode ok" if task["state"] == "done" else f"transcode {task['state']}"
            time.sleep(1)
    code = (body or {}).get("error", {}).get("code", "")
    return f"error {status} {code}".strip()


def probes(data_dir, ids):
    """Stored ffprobe results, read-only. The API does not expose them."""
    uri = f"file:{data_dir}/neo.sqlite3?mode=ro"
    with sqlite3.connect(uri, uri=True) as db:
        rows = db.execute(
            f"SELECT id, probe FROM tracks WHERE id IN ({','.join('?' * len(ids))})", ids)
        return {i: json.loads(p) if p else None for i, p in rows}


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--url", required=True, help="the running server, e.g. http://127.0.0.1:8888")
    parser.add_argument("--user", required=True)
    parser.add_argument("--password-stdin", action="store_true")
    parser.add_argument("--data-dir", help="read stored probes for codec details")
    parser.add_argument("--per-extension", type=int, default=20)
    parser.add_argument("--can-play", action="append",
                        help="a MIME type the browser plays; repeat. Defaults to Android Chrome")
    parser.add_argument("--seed", type=int, default=0)
    args = parser.parse_args()

    password = sys.stdin.readline().rstrip("\n") if args.password_stdin else getpass.getpass()
    api = Api(args.url)
    status, body = api.call("POST", "/api/auth/login", {"name": args.user, "password": password})
    if status != 200:
        sys.exit(f"login failed: {status}")
    if not body["user"]["is_admin"]:
        sys.exit("the survey cancels transcodes, which needs an administrator")
    api.token = body["token"]
    can_play = args.can_play or ANDROID_CHROME

    by_ext = playable_tracks(api)
    rng = random.Random(args.seed)
    decisions = {}
    for ext, ids in sorted(by_ext.items()):
        finished = False
        for track_id in rng.sample(ids, min(args.per_extension, len(ids))):
            decision = decide(api, track_id, can_play, not finished)
            finished = finished or decision.startswith("transcode ")
            decisions[track_id] = (ext, decision)

    probed = probes(args.data_dir, list(decisions)) if args.data_dir and decisions else {}
    groups = collections.Counter()
    for track_id, (ext, decision) in decisions.items():
        probe = probed.get(track_id) or {}
        audio = probe.get("audio") or {}
        video = probe.get("video") or {}
        key = (ext, probe.get("format", "?"), audio.get("codec", "-"),
               audio.get("profile") or "-", video.get("codec", "-"), decision)
        groups[key] += 1

    report = {
        "can_play": can_play,
        "tracks_by_extension": {ext: len(ids) for ext, ids in sorted(by_ext.items())},
        "sampled": len(decisions),
        "groups": [
            dict(zip(("extension", "format", "audio_codec", "profile", "video_codec",
                      "decision", "count"), (*key, n)))
            for key, n in sorted(groups.items())
        ],
    }
    json.dump(report, sys.stdout, indent=2, ensure_ascii=False)
    print()


if __name__ == "__main__":
    main()
