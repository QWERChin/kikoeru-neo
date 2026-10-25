#!/usr/bin/env python3
"""Resource benchmark for kikoeru-neo on synthetic data.

Generates a library of short ffmpeg-made tracks, runs the real server
binary against it, and reports memory, scan time, API latency under load,
cache eviction and leftovers. book/src/testing.md "资源基准" explains how
to run it and how to read the report. Only the standard library is used,
so it runs on a bare host with python3 and ffmpeg.
"""

import argparse
import json
import os
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request

ADMIN = ("bench", "bench password")
# Long enough that a transcode takes a while and its output is a few MB,
# so a handful of them pass the small cache limit and force eviction.
LONG_SECONDS = 120
SAMPLE_INTERVAL = 0.1


def run(cmd, **kw):
    return subprocess.run(cmd, check=True, capture_output=True, **kw)


def generate(cmd_out, seconds, args):
    run(["ffmpeg", "-nostdin", "-loglevel", "error", "-f", "lavfi", "-i",
         f"sine=frequency=440:duration={seconds}", *args, cmd_out])


def make_library(root, works, tracks_per_work, long_tracks):
    """Short tracks are hard links of one file, so a large library costs
    little disk. Each path is still its own track to the scanner."""
    os.makedirs(root)
    short = os.path.join(root, ".short.mp3")
    generate(short, 2, ["-c:a", "libmp3lame", "-b:a", "64k"])
    for w in range(works):
        work = os.path.join(root, f"RJ{w + 1:08d}", "mp3")
        os.makedirs(work)
        for t in range(tracks_per_work):
            os.link(short, os.path.join(work, f"{t + 1:02d}.mp3"))
    os.unlink(short)
    # Distinct long files, so each transcode has its own cache key.
    longs = os.path.join(root, f"RJ{works + 1:08d}")
    os.makedirs(longs)
    for i in range(long_tracks):
        generate(os.path.join(longs, f"long{i:02d}.flac"), LONG_SECONDS + i, ["-c:a", "flac"])


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def write_config(path, port, root, max_bytes, min_free_bytes):
    with open(path, "w") as f:
        f.write(f'listen = "127.0.0.1:{port}"\n\n')
        f.write(f"[cache]\nmax_bytes = {max_bytes}\nmin_free_bytes = {min_free_bytes}\n\n")
        f.write("[media]\nheavy_jobs = 2\n\n")
        # The synthetic works have no DLsite pages to fetch.
        f.write('[metadata]\nsource = "none"\n\n')
        f.write(f'[[roots]]\ntype = "local"\nid = "bench"\ndisplay_name = "Bench"\npath = "{root}"\n')


class Server:
    def __init__(self, binary, data_dir, config, port):
        self.base = f"http://127.0.0.1:{port}"
        self.log = open(os.path.join(os.path.dirname(data_dir), "server.log"), "ab")
        self.proc = subprocess.Popen(
            [binary, "--data-dir", data_dir, "serve", "--config", config],
            stdout=self.log, stderr=self.log,
            env={**os.environ, "RUST_LOG": "warn"},
            # Its own process group, which ffmpeg and ffprobe inherit, so
            # cleanup can reach them even after the server itself is gone.
            start_new_session=True)
        self.token = None
        deadline = time.time() + 30
        try:
            while time.time() < deadline:
                try:
                    if self.request("GET", "/api/ready")[0] == 200:
                        return
                except OSError:
                    pass
                time.sleep(0.2)
            raise RuntimeError("server did not become ready; see server.log")
        except BaseException:
            # The caller never gets this object, so nothing else would stop it.
            self.kill_group()
            self.log.close()
            raise

    def request(self, method, path, body=None, headers=None, read=True):
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(self.base + path, data=data, method=method)
        req.add_header("content-type", "application/json")
        if self.token:
            req.add_header("authorization", f"Bearer {self.token}")
        for k, v in (headers or {}).items():
            req.add_header(k, v)
        try:
            with urllib.request.urlopen(req, timeout=60) as resp:
                payload = resp.read() if read else b""
                return resp.status, payload
        except urllib.error.HTTPError as e:
            return e.code, e.read()

    def json(self, method, path, body=None):
        status, payload = self.request(method, path, body)
        return status, (json.loads(payload) if payload else None)

    def login(self):
        status, body = self.json("POST", "/api/auth/login",
                                 {"name": ADMIN[0], "password": ADMIN[1]})
        assert status == 200, body
        self.token = body["token"]

    def wait_task(self, task_id, timeout=3600):
        deadline = time.time() + timeout
        while time.time() < deadline:
            _, task = self.json("GET", f"/api/tasks/{task_id}")
            if task["state"] not in ("queued", "running"):
                return task
            time.sleep(0.2)
        raise RuntimeError(f"task {task_id} did not finish")

    def kill_group(self):
        try:
            os.killpg(self.proc.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        self.proc.wait()

    def stop(self):
        """Stops the server and returns its exit status. Anything but 0
        means it crashed, was killed or failed to shut down cleanly."""
        if self.proc.poll() is None:
            self.proc.send_signal(signal.SIGTERM)
        status = self.proc.wait(timeout=30)
        self.log.close()
        return status


def rss_kib(pid, field="VmRSS"):
    try:
        with open(f"/proc/{pid}/status") as f:
            for line in f:
                if line.startswith(field + ":"):
                    return int(line.split()[1])
    except OSError:
        pass
    return 0


def children(pid):
    found = []
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        try:
            with open(f"/proc/{entry}/stat") as f:
                fields = f.read().rsplit(")", 1)[1].split()
            if int(fields[1]) == pid:
                found.append(int(entry))
        except OSError:
            continue
    return found


def comm(pid):
    try:
        with open(f"/proc/{pid}/comm") as f:
            return f.read().strip()
    except OSError:
        return ""


def processes_using(path):
    """Processes that mention `path` in their command line or hold a file
    under it open. ffmpeg and ffprobe get local files on stdin as
    `/dev/fd/0`, so the command line alone would miss them. After the server
    has exited, any such process is an orphan."""
    found = []
    for entry in os.listdir("/proc"):
        if not entry.isdigit() or int(entry) == os.getpid():
            continue
        try:
            with open(f"/proc/{entry}/cmdline", "rb") as f:
                hit = path.encode() in f.read()
            fd_dir = f"/proc/{entry}/fd"
            for fd in [] if hit else os.listdir(fd_dir):
                if os.readlink(os.path.join(fd_dir, fd)).startswith(path):
                    hit = True
                    break
        except OSError:
            continue
        if hit:
            found.append(int(entry))
    return found


class Sampler(threading.Thread):
    """Peak RSS of the server and of all its ffmpeg and ffprobe children
    together. Probes of short files can finish between two samples."""

    def __init__(self, pid):
        super().__init__(daemon=True)
        self.pid = pid
        self.stop_event = threading.Event()
        self.server_peak = 0
        self.children_peak = 0
        self.max_children = 0

    def run(self):
        while not self.stop_event.is_set():
            self.server_peak = max(self.server_peak, rss_kib(self.pid))
            kids = children(self.pid)
            self.max_children = max(self.max_children, len(kids))
            self.children_peak = max(self.children_peak, sum(rss_kib(k) for k in kids))
            time.sleep(SAMPLE_INTERVAL)

    def finish(self):
        self.stop_event.set()
        self.join()


def streamer(server, track_ids, stop, errors):
    """Plays like a browser: Range requests of 256 KiB, walking the file.
    A request that raises counts as a failure, so a worker that loses its
    connection cannot quietly drop out of the load."""
    i = 0
    while not stop.is_set():
        try:
            track = track_ids[i % len(track_ids)]
            status, _ = server.request("GET", f"/api/tracks/{track}/stream",
                                       headers={"range": "bytes=0-262143"})
            if status not in (200, 206):
                errors.append(status)
        except Exception as e:  # noqa: BLE001 - any failure is a result
            errors.append(repr(e))
        i += 1


def latency_probe(server, stop, samples, errors):
    page = 1
    while not stop.is_set():
        start = time.perf_counter()
        try:
            status, _ = server.request("GET", f"/api/works?page={page}&page_size=40")
            samples.append((time.perf_counter() - start) * 1000)
            if status != 200:
                errors.append(status)
        except Exception as e:  # noqa: BLE001 - any failure is a result
            errors.append(repr(e))
        page = page % 20 + 1
        time.sleep(0.05)


def percentile(values, p):
    if not values:
        return None
    ordered = sorted(values)
    return round(ordered[min(len(ordered) - 1, int(len(ordered) * p))], 1)


def usage(path):
    """File lengths, which is what eviction compares with max_bytes, and the
    blocks actually allocated on disk."""
    length = allocated = 0
    for dirpath, _, files in os.walk(path):
        for name in files:
            try:
                st = os.lstat(os.path.join(dirpath, name))
            except OSError:
                continue
            length += st.st_size
            allocated += st.st_blocks * 512
    return length, allocated


def scan_summary(data, task_id):
    """The API exposes only transcode results, so the scan counters are
    read from the task row, read-only."""
    import sqlite3
    uri = f"file:{os.path.join(data, 'neo.sqlite3')}?mode=ro"
    with sqlite3.connect(uri, uri=True) as db:
        (result,) = db.execute("SELECT result FROM tasks WHERE id = ?", (task_id,)).fetchone()
    summary = json.loads(result)
    summary.pop("log", None)
    return summary


def all_tracks(server):
    tracks, longs = [], []
    page = 1
    while True:
        _, body = server.json("GET", f"/api/works?page={page}&page_size=100")
        for work in body["items"]:
            _, detail = server.json("GET", f"/api/works/{work['id']}")
            for t in detail["tracks"]:
                (longs if t["path"].endswith(".flac") else tracks).append(t["id"])
        if page * 100 >= body["total"]:
            return tracks, longs
        page += 1


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--bin", default="./result/bin/kikoeru-neo")
    parser.add_argument("--works", type=int, default=3000)
    parser.add_argument("--tracks", type=int, default=8, help="tracks per work")
    parser.add_argument("--long-tracks", type=int, default=7,
                        help="flac files to transcode; the last one is kept for the low disk check")
    parser.add_argument("--streams", type=int, default=4, help="concurrent players")
    parser.add_argument("--load-seconds", type=int, default=30)
    parser.add_argument("--cache-max-bytes", type=int, default=8_000_000)
    parser.add_argument("--keep", action="store_true", help="keep the work directory")
    args = parser.parse_args()
    # Every phase needs something to do, or it would pass without measuring.
    for name, least in (("works", 1), ("tracks", 1), ("long_tracks", 2), ("streams", 1)):
        if getattr(args, name) < least:
            parser.error(f"--{name.replace('_', '-')} must be at least {least}")
    binary = os.path.abspath(args.bin)
    # SIGTERM (from `timeout`, say) would end Python without running the
    # cleanup below, and the server, in its own session, would live on.
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))

    work = tempfile.mkdtemp(prefix="kikoeru-bench-")
    os.chmod(work, 0o700)
    report = {"host": os.uname().nodename, "cpus": os.cpu_count(), "args": vars(args)}
    # Everything started is stopped in `finally`, before the directory it
    # uses is removed, even when a step raises.
    servers, stops = [], []
    try:
        root = os.path.join(work, "library")
        data = os.path.join(work, "data")
        config = os.path.join(work, "config.toml")
        t0 = time.time()
        make_library(root, args.works, args.tracks, args.long_tracks)
        report["library_seconds"] = round(time.time() - t0, 1)

        port = free_port()
        write_config(config, port, root, args.cache_max_bytes, 0)
        run([binary, "--data-dir", data, "user", "create", "--admin", ADMIN[0],
             "--password-stdin"], input=(ADMIN[1] + "\n").encode())
        server = Server(binary, data, config, port)
        servers.append(server)
        server.login()
        time.sleep(2)
        report["rss_idle_kib"] = rss_kib(server.proc.pid)

        # Full scan of an empty database.
        start = time.time()
        _, scan = server.json("POST", "/api/admin/scan", {"root": "bench"})
        task = server.wait_task(scan["task_ids"][0])
        report["scan_state"] = task["state"]
        report["scan_seconds"] = round(time.time() - start, 1)
        report["scan_result"] = scan_summary(data, task["id"])
        report["rss_after_scan_kib"] = rss_kib(server.proc.pid)
        tracks, longs = all_tracks(server)
        # A scan that skipped files would leave the load phases with less
        # to do than they report.
        if len(tracks) != args.works * args.tracks or len(longs) != args.long_tracks:
            raise RuntimeError(f"scan found {len(tracks)} short and {len(longs)} long tracks, "
                               f"expected {args.works * args.tracks} and {args.long_tracks}")

        # Streaming only.
        stop, errors = threading.Event(), []
        stops.append(stop)
        sampler = Sampler(server.proc.pid)
        sampler.start()
        players = [threading.Thread(target=streamer, args=(server, tracks, stop, errors),
                                    daemon=True)
                   for _ in range(args.streams)]
        for p in players:
            p.start()
        time.sleep(min(10, args.load_seconds))
        report["rss_streaming_kib"] = rss_kib(server.proc.pid)
        streaming_errors = errors
        stop.set()
        for p in players:
            p.join()
        sampler.finish()
        report["streaming_errors"] = len(streaming_errors)

        # Combined load: players, a rescan, transcodes past the cache limit,
        # and API latency sampled throughout.
        stop, errors, latencies = threading.Event(), [], []
        stops.append(stop)
        sampler = Sampler(server.proc.pid)
        sampler.start()
        threads = [threading.Thread(target=streamer, args=(server, tracks, stop, errors),
                                    daemon=True)
                   for _ in range(args.streams)]
        threads.append(threading.Thread(target=latency_probe,
                                        args=(server, stop, latencies, errors), daemon=True))
        for t in threads:
            t.start()
        start = time.time()
        _, rescan = server.json("POST", "/api/admin/scan", {"root": "bench"})
        transcodes, prepare_failures = [], []
        # The last long track is kept out of the cache for the low disk check.
        for track in longs[:-1]:
            status, body = server.json("POST", f"/api/tracks/{track}/prepare",
                                       {"can_play": [], "force_fallback": True})
            if status == 202:
                transcodes.append(body["task_id"])
            else:
                prepare_failures.append(status)
        ended = [server.wait_task(t) for t in transcodes]
        rescan_task = server.wait_task(rescan["task_ids"][0])
        rescan_failed = scan_summary(data, rescan_task["id"]).get("works_failed")
        remaining = args.load_seconds - (time.time() - start)
        if remaining > 0:
            time.sleep(remaining)
        stop.set()
        for t in threads:
            t.join()
        sampler.finish()
        report["combined"] = {
            "seconds": round(time.time() - start, 1),
            "rescan_state": rescan_task["state"],
            "rescan_works_failed": rescan_failed,
            "prepare_failures": prepare_failures,
            "transcodes": {s: sum(1 for t in ended if t["state"] == s)
                           for s in {t["state"] for t in ended}},
            "api_requests": len(latencies),
            "api_p50_ms": percentile(latencies, 0.50),
            "api_p95_ms": percentile(latencies, 0.95),
            "errors": len(errors),
            "server_peak_rss_kib": sampler.server_peak,
            "children_peak_rss_kib": sampler.children_peak,
            "children_max_concurrent": sampler.max_children,
        }
        cache = os.path.join(data, "cache")
        report["cache"] = {
            "transcode_bytes": usage(os.path.join(cache, "transcode"))[0],
            "transcode_allocated_bytes": usage(os.path.join(cache, "transcode"))[1],
            "max_bytes": args.cache_max_bytes,
            "files": len(os.listdir(os.path.join(cache, "transcode"))),
            "evicted": sum(1 for t in ended if t["state"] == "done")
            - len(os.listdir(os.path.join(cache, "transcode"))),
            "tmp_leftovers": os.listdir(os.path.join(cache, "tmp")),
        }
        # Background probes from the scan may still run; transcodes may not.
        kids = [comm(k) for k in children(server.proc.pid)]
        report["after_load"] = {
            "ffmpeg_running": kids.count("ffmpeg"),
            "ffprobe_running": kids.count("ffprobe"),
        }

        # Read before the restart below replaces the process.
        report["server_hwm_kib"] = rss_kib(server.proc.pid, "VmHWM")

        # Low disk: with min_free_bytes above the free space, a new
        # transcode is refused up front and leaves nothing behind.
        report["server_exits"] = [server.stop()]
        free = shutil.disk_usage(data).free
        write_config(config, port, root, args.cache_max_bytes, free + 10**9)
        server = Server(binary, data, config, port)
        servers.append(server)
        server.login()
        status, _ = server.json("POST", f"/api/tracks/{longs[-1]}/prepare",
                                {"can_play": [], "force_fallback": True})
        report["low_disk"] = {
            "prepare_status": status,
            "tmp_leftovers": os.listdir(os.path.join(cache, "tmp")),
        }
        report["server_exits"].append(server.stop())
        report["orphans_after_shutdown"] = len(processes_using(work))
    finally:
        for stop in stops:
            stop.set()
        for server in servers:
            if server.proc.poll() is None:
                server.proc.terminate()
                try:
                    server.proc.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    pass
            # Media children can outlive a server that crashed or was
            # killed. Orphans were already counted above on the normal path.
            server.kill_group()
        if args.keep:
            print(f"kept {work}", file=sys.stderr)
        else:
            shutil.rmtree(work, ignore_errors=True)

    json.dump(report, sys.stdout, indent=2, ensure_ascii=False)
    print()
    failed = []
    if report["cache"]["tmp_leftovers"] or report["low_disk"]["tmp_leftovers"]:
        failed.append("half-written files left in cache/tmp")
    if report["after_load"]["ffmpeg_running"]:
        failed.append("ffmpeg still running after every transcode ended")
    if report["orphans_after_shutdown"]:
        failed.append("ffmpeg or ffprobe outlived the server")
    if report["cache"]["transcode_bytes"] > args.cache_max_bytes:
        failed.append("cache/transcode is over cache.max_bytes")
    if report["low_disk"]["prepare_status"] != 507:
        failed.append("prepare was not refused on low disk")
    if report["cache"]["evicted"] <= 0:
        failed.append("the transcodes never passed cache.max_bytes, so eviction was not exercised")
    if report["scan_state"] != "done" or report["combined"]["rescan_state"] != "done":
        failed.append("a scan did not finish as done")
    if report["scan_result"].get("works_failed") or report["combined"]["rescan_works_failed"]:
        failed.append("a scan could not list some works")
    if report["combined"]["prepare_failures"]:
        failed.append("a prepare request did not start a transcode")
    if set(report["combined"]["transcodes"]) - {"done"}:
        failed.append("a transcode did not finish as done")
    if any(report["server_exits"]):
        failed.append("the server did not exit cleanly (crash, OOM or failed shutdown)")
    if report["streaming_errors"]:
        failed.append("requests failed while only streaming")
    if report["combined"]["errors"]:
        failed.append("requests failed under load")
    for line in failed:
        print(f"FAIL: {line}", file=sys.stderr)
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
