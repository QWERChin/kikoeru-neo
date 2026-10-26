#!/usr/bin/env python3
"""Soak test against a running kikoeru-neo with a disposable synthetic library.

Virtual listeners stream tracks at real-time rate with seeks and track
switches and fetch each track's loudness and waveform. One of them forces
a transcode now and then, one plays a video through HLS, and a rescan runs
halfway. Every 30 s the server's memory, file descriptors, threads, child
processes, cache size and API latency are sampled. book/src/testing.md
"资源预算" explains how to run it and read the report. Only the standard
library is used.

The report holds counts and timings only, never titles or paths. Playback
stops at once when the service log shows the provider throttling.
"""

import argparse
import http.client
import json
import math
import os
import random
import statistics
import subprocess
import sys
import threading
import time
import tomllib
import urllib.error
import urllib.request

_random = threading.local()


def random_source():
    return getattr(_random, "source", random)


CHUNK = 256 * 1024
AUDIO_CAN_PLAY = ["audio/mpeg", "audio/mp4", "audio/aac", "audio/wav", "audio/flac", "audio/ogg"]
# Without video/mp4, an mp4 video is served as HLS.
HLS_CAN_PLAY = AUDIO_CAN_PLAY
# Bytes per second when the duration is not known yet.
RATE_BY_EXT = {"mp3": 40_000, "m4a": 32_000, "flac": 110_000, "ogg": 24_000, "wav": 200_000}


class Api:
    def __init__(self, base, stats):
        self.base = base.rstrip("/")
        self.token = None
        self.stats = stats
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def request(self, method, path, body=None, headers=None, raw=False, timeout=120, want_headers=False):
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(self.base + path, data=data, method=method)
        if data is not None:
            req.add_header("content-type", "application/json")
        if self.token:
            req.add_header("authorization", f"Bearer {self.token}")
        for key, value in (headers or {}).items():
            req.add_header(key, value)
        try:
            with self.opener.open(req, timeout=timeout) as resp:
                payload = resp.read()
                status, reply_headers = resp.status, resp.headers
        except urllib.error.HTTPError as e:
            payload, status, reply_headers = e.read(), e.code, e.headers
        except (OSError, http.client.HTTPException):
            # Timeouts, resets and cut bodies would otherwise end the worker
            # thread and quietly lower the load. Status 0 marks them.
            payload, status, reply_headers = b"", 0, {}
        self.stats.status(status)
        if want_headers:
            return status, payload, reply_headers
        if raw:
            return status, payload
        try:
            return status, json.loads(payload) if payload else None
        except ValueError:
            return status, None


class Stats:
    def __init__(self):
        self.lock = threading.Lock()
        self.codes = {}
        self.latencies = []
        self.events = {}
        self.tasks = set()
        # Set by the stop's cancel round. A request still in flight then can
        # answer with a task afterwards, which is cancelled as it arrives.
        self.cancelling = None

    def status(self, code):
        with self.lock:
            self.codes[code] = self.codes.get(code, 0) + 1

    def latency(self, ms):
        with self.lock:
            self.latencies.append(ms)

    def take_latencies(self):
        with self.lock:
            taken, self.latencies = self.latencies, []
        return taken

    def event(self, name):
        with self.lock:
            self.events[name] = self.events.get(name, 0) + 1

    def task(self, task_id):
        """Remembers a server task this run started, to cancel it on a stop."""
        with self.lock:
            api = self.cancelling
            if api is None:
                self.tasks.add(task_id)
                return
        api.request("POST", f"/api/tasks/{task_id}/cancel")

    def cancel_from_now(self, api):
        """Returns the tasks started so far and has later ones cancelled as
        they register, in one step, so none falls between the two."""
        with self.lock:
            self.cancelling = api
            taken, self.tasks = self.tasks, set()
        return taken


def p95(values):
    """Nearest-rank p95, so a window of a few probes keeps its slowest."""
    return percentile(values, 0.95)


def percentile(values, fraction):
    if not values:
        return None
    values = sorted(values)
    return round(values[math.ceil(len(values) * fraction) - 1], 3)


class Library:
    """Random tracks from one root, found through the API."""

    def __init__(self, api, root, stop):
        self.api, self.root, self.stop = api, root, stop
        status, page = api.request("GET", f"/api/works?root={root}&page=1&page_size=1")
        if status != 200:
            sys.exit(f"cannot list works of {root}: {status}")
        self.total = page["total"]

    def tracks(self, kind, wanted=lambda _track: True):
        for _ in range(50):
            if self.stop.is_set():
                return []
            page = random_source().randint(1, max(1, self.total))
            status, body = self.api.request("GET", f"/api/works?root={self.root}&page={page}&page_size=1")
            if status != 200 or not body["items"]:
                continue
            status, work = self.api.request("GET", f"/api/works/{body['items'][0]['id']}")
            if status != 200:
                continue
            found = [t for t in work["tracks"] if t["kind"] == kind and not t["missing"] and wanted(t)]
            if found:
                return found
        return []


def rate_of(track):
    if track.get("size") and track.get("duration_ms"):
        return max(8_000, track["size"] * 1000 // track["duration_ms"])
    ext = track["path"].rsplit(".", 1)[-1].lower()
    return RATE_BY_EXT.get(ext, 64_000)


def play(api, stop, stats, url, track, seconds):
    """Reads `url` with Range requests at the track's rate for `seconds`,
    seeking now and then."""
    rate = rate_of(track)
    # A transcoded file is smaller than the track, so seeks use the length
    # the server reports rather than the track's size.
    size = None
    pos = 0
    end = time.monotonic() + seconds
    next_seek = time.monotonic() + random_source().uniform(60, 180)
    while time.monotonic() < end and not stop.is_set():
        started = time.monotonic()
        status, body, headers = api.request("GET", url, headers={"range": f"bytes={pos}-{pos + CHUNK - 1}"},
                                            want_headers=True)
        if status not in (200, 206):
            stats.event(f"stream_{status}")
            return
        total = headers.get("content-range", "").rpartition("/")[2]
        if total.isdigit() and size is None:
            size = int(total)
            # A transcode is AAC at its own bitrate, so pace by what is served.
            if track.get("duration_ms"):
                rate = max(8_000, size * 1000 // track["duration_ms"])
        pos += len(body)
        # The last chunk is paced too, so a short clip still takes its time.
        stop.wait(max(0.0, len(body) / rate - (time.monotonic() - started)))
        if size and pos >= size:
            return
        if time.monotonic() >= next_seek and size:
            pos = random_source().randrange(0, size)
            stats.event("seek")
            next_seek = time.monotonic() + random_source().uniform(60, 180)


def wait_task(api, stop, task_id, limit=1800):
    end = time.monotonic() + limit
    while time.monotonic() < end and not stop.is_set():
        status, task = api.request("GET", f"/api/tasks/{task_id}")
        if status == 200 and task["state"] not in ("queued", "running"):
            return task
        stop.wait(2)
    return None


def prepare(api, stop, track, can_play, force=False):
    status, body = api.request("POST", f"/api/tracks/{track['id']}/prepare",
                               {"can_play": can_play, "force_fallback": force})
    if status == 200:
        return body["url"]
    if status == 202:
        api.stats.task(body["task_id"])
        task = wait_task(api, stop, body["task_id"])
        if task and task["state"] == "done" and task.get("result"):
            return task["result"]["url"]
        if task and task["state"] != "done":
            # A simultaneous stop must not hide a terminal task failure.
            api.stats.event(f"prepare_task_{task['state']}")
    return None


def analyse(api, stop, stats, track):
    """Asks for loudness and the waveform the way the player does, waiting
    for the analysis task the first request queues, then revalidates the
    waveform with its ETag."""
    path = f"/api/tracks/{track['id']}"
    status, body = api.request("GET", f"{path}/loudness")
    if status == 202:
        stats.task(body["task_id"])
        task = wait_task(api, stop, body["task_id"])
        if task is None and stop.is_set():
            stats.event("analysis_interrupted")
            return
        if not task or task["state"] != "done":
            stats.event("analysis_failed")
            return
        stats.event("analysis")
        if stop.is_set():
            return
        status, body = api.request("GET", f"{path}/loudness")
    if status != 200:
        stats.event(f"loudness_{status}")
        return
    status, body, headers = api.request("GET", f"{path}/waveform", want_headers=True)
    if status == 202:
        # An evicted waveform is queued again while loudness stays known.
        task_id = json.loads(body)["task_id"]
        stats.task(task_id)
        task = wait_task(api, stop, task_id)
        if task is None and stop.is_set():
            stats.event("waveform_interrupted")
            return
        if not task or task["state"] != "done":
            stats.event("waveform_regeneration_failed")
            return
        stats.event("waveform_regenerated")
        if stop.is_set():
            return
        status, _, headers = api.request("GET", f"{path}/waveform", want_headers=True)
    stats.event(f"waveform_{status}")
    if status == 200 and headers.get("etag"):
        status, _ = api.request("GET", f"{path}/waveform", headers={"if-none-match": headers["etag"]}, raw=True)
        stats.event(f"waveform_{status}")


def listener(api, lib, stop, stats, transcodes):
    last_transcode = time.monotonic()
    while not stop.is_set():
        tracks = lib.tracks("audio")
        if not tracks:
            stop.wait(30)
            continue
        if stop.is_set():
            return
        track = random_source().choice(tracks)
        force = transcodes and time.monotonic() - last_transcode > 15 * 60
        url = prepare(api, stop, track, AUDIO_CAN_PLAY, force)
        if stop.is_set():
            return
        if force:
            last_transcode = time.monotonic()
            stats.event("transcode" if url else "transcode_failed")
        if url is None:
            stats.event("prepare_failed")
            stop.wait(30)
            continue
        stats.event("track")
        # The player fetches these without holding up playback.
        threading.Thread(target=guarded, args=(analyse, stop, stats, api, stop, stats, track),
                         daemon=True).start()
        # A track switch every 4 to 10 minutes keeps link fetches far apart.
        play(api, stop, stats, url, track, random_source().uniform(240, 600))


def video(api, lib, stop, stats):
    while not stop.is_set():
        tracks = lib.tracks("video", lambda t: t["path"].lower().endswith(".mp4")
                            and (t.get("size") or 0) < 1_000_000_000)
        if not tracks:
            stop.wait(60)
            continue
        if stop.is_set():
            return
        url = prepare(api, stop, random_source().choice(tracks), HLS_CAN_PLAY)
        if stop.is_set():
            return
        if not url or not url.endswith(".m3u8"):
            stats.event("hls_failed")
            stop.wait(60)
            continue
        stats.event("hls")
        status, playlist = api.request("GET", url, raw=True)
        if status != 200:
            continue
        base = url.rsplit("/", 1)[0]
        lines = playlist.decode().splitlines()
        segments = [(float(lines[i].split(":")[1].rstrip(",")), lines[i + 1])
                    for i, line in enumerate(lines) if line.startswith("#EXTINF") and i + 1 < len(lines)]
        end = time.monotonic() + random_source().uniform(600, 1200)
        i = 0
        while i < len(segments) and time.monotonic() < end and not stop.is_set():
            length, uri = segments[i]
            status, _ = api.request("GET", f"{base}/{uri}", raw=True)
            if status != 200:
                stats.event(f"segment_{status}")
                break
            i += random_source().randrange(5, 40) if random_source().random() < 0.02 else 1
            stop.wait(length)


def latency_probe(api, stop, stats):
    while not stop.is_set():
        started = time.monotonic()
        api.request("GET", "/api/works?page=1")
        stats.latency((time.monotonic() - started) * 1000)
        stop.wait(5)


def guarded(target, stop, stats, *args, seed=None):
    """Runs a worker loop again after an unexpected error, so the load does
    not shrink without the report saying so."""
    if seed is not None:
        _random.source = random.Random(seed)
    while not stop.is_set():
        try:
            target(*args)
            return
        except Exception as e:  # noqa: BLE001 - every failure is counted
            stats.event(f"worker_error_{type(e).__name__}")
            stop.wait(30)


def descendants(pid):
    children = {}
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        try:
            with open(f"/proc/{entry}/stat") as f:
                fields = f.read().rsplit(")", 1)[1].split()
            with open(f"/proc/{entry}/comm") as f:
                comm = f.read().strip()
        except OSError:
            continue
        children.setdefault(int(fields[1]), []).append((int(entry), comm))
    found, queue = [], [pid]
    while queue:
        for child in children.get(queue.pop(), []):
            found.append(child)
            queue.append(child[0])
    return found


def process_sample(pid):
    status = {}
    with open(f"/proc/{pid}/status") as f:
        for line in f:
            key, _, value = line.partition(":")
            status[key] = value.strip()
    kids = descendants(pid)
    with open(f"/proc/{pid}/stat") as f:
        fields = f.read().rsplit(")", 1)[1].split()
    live_child_ticks = 0
    for child, _ in kids:
        try:
            with open(f"/proc/{child}/stat") as f:
                child_fields = f.read().rsplit(")", 1)[1].split()
            live_child_ticks += sum(int(v) for v in child_fields[11:15])
        except FileNotFoundError:
            pass
    rss_kib = int(status["VmRSS"].split()[0])
    swap_kib = int(status["VmSwap"].split()[0])
    return {
        "rss_mib": round(rss_kib / 1024, 1),
        "swap_mib": round(swap_kib / 1024, 1),
        # Paging out retained buffers is not allocator release.
        "rss_plus_swap_mib": round((rss_kib + swap_kib) / 1024, 1),
        "hwm_mib": round(int(status["VmHWM"].split()[0]) / 1024, 1),
        "cpu_seconds": (int(fields[11]) + int(fields[12])) / os.sysconf("SC_CLK_TCK"),
        "reaped_children_cpu_seconds": (int(fields[13]) + int(fields[14])) / os.sysconf("SC_CLK_TCK"),
        "live_children_cpu_seconds": live_child_ticks / os.sysconf("SC_CLK_TCK"),
        "start_ticks": int(fields[19]),
        "loadavg": list(os.getloadavg()),
        "threads": int(status["Threads"]),
        "fds": len(os.listdir(f"/proc/{pid}/fd")),
        "children": len(kids),
        "ffmpeg": sum(1 for _, comm in kids if comm.startswith("ffmpeg") or comm.startswith("ffprobe")),
    }


def cache_bytes(data_dir):
    # Transcodes, HLS sets and waveforms all live under cache/transcode.
    total = 0
    for root, _, files in os.walk(os.path.join(data_dir, "cache", "transcode")):
        for name in files:
            try:
                total += os.path.getsize(os.path.join(root, name))
            except OSError:
                pass
    return total


def throttled_since(unit, since):
    """True or False from the unit's log, or None when the log cannot be
    read, because then the throttle guard is blind."""
    try:
        done = subprocess.run(["journalctl", "--user", "-u", unit, "--since", since, "-o", "cat",
                               "--no-pager"], capture_output=True, text=True, timeout=60)
    except (OSError, subprocess.TimeoutExpired):
        return None
    if done.returncode != 0:
        return None
    return "provider is throttling" in done.stdout


def cancel_tasks(api, stats):
    """Cancels the server tasks this run started. Finished ones answer 400
    and unknown ones 404, which is fine."""
    for task_id in stats.cancel_from_now(api):
        api.request("POST", f"/api/tasks/{task_id}/cancel")


def slope_per_hour(points):
    """Least-squares slope of (seconds, value) points, per hour."""
    if len(points) < 3:
        return None
    xs, ys = [p[0] for p in points], [p[1] for p in points]
    mx, my = statistics.mean(xs), statistics.mean(ys)
    den = sum((x - mx) ** 2 for x in xs)
    return round(sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / den * 3600, 2) if den else None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", required=True)
    parser.add_argument("--user", required=True)
    parser.add_argument("--password-file", required=True, help="read by this program, never printed")
    parser.add_argument("--data-dir", required=True, help="the server's data directory, read only")
    process = parser.add_mutually_exclusive_group(required=True)
    process.add_argument("--unit", help="the server's user systemd unit")
    process.add_argument("--pid", type=int, help="sample this isolated server instead of a unit")
    parser.add_argument("--log-file", help="server log required with --pid for the throttle guard")
    parser.add_argument("--root", required=True, action="append",
                        help="repeat to distribute listeners across roots")
    parser.add_argument("--rescan-root", help="scanned once halfway through")
    parser.add_argument("--listeners", type=int, default=3)
    parser.add_argument("--hours", type=float, default=3.5)
    parser.add_argument("--warmup-minutes", type=float, default=20)
    parser.add_argument("--sample-seconds", type=float, default=30)
    parser.add_argument("--cooldown-seconds", type=float, default=60)
    parser.add_argument("--seed", type=int)
    args = parser.parse_args()
    if args.pid is not None and (args.pid <= 0 or not args.log_file):
        parser.error("--pid must be positive and requires --log-file")
    if args.listeners < len(args.root) or args.hours <= 0 or args.sample_seconds <= 0 or args.cooldown_seconds < 0:
        parser.error("each root needs a listener, durations must be positive, and cooldown nonnegative")
    random.seed(args.seed)

    stats = Stats()
    api = Api(args.url, stats)
    with open(args.password_file) as f:
        password = f.readline().rstrip("\r\n")
    status, body = api.request("POST", "/api/auth/login", {"name": args.user, "password": password})
    del password
    if status != 200:
        sys.exit(f"login failed: {status}")
    api.token = body["token"]
    pid = args.pid if args.pid is not None else int(subprocess.run(
        ["systemctl", "--user", "show", "-p", "MainPID", "--value", args.unit],
        capture_output=True, text=True, check=True).stdout.strip())
    if pid == 0:
        sys.exit(f"{args.unit} is not running")
    with open(os.path.join(args.data_dir, "config.toml"), "rb") as f:
        budget = tomllib.load(f).get("cache", {}).get("max_bytes", "20GB")
    since = time.strftime("%Y-%m-%d %H:%M:%S")
    def check_throttle():
        if args.pid is None:
            return throttled_since(args.unit, since)
        try:
            with open(args.log_file) as log:
                return "provider is throttling" in log.read()
        except OSError:
            return None

    if check_throttle() is None:
        sys.exit("cannot read the server log, so throttling would go unnoticed")
    stop = threading.Event()
    libs = [Library(api, root, stop) for root in args.root]
    if any(lib.total == 0 for lib in libs):
        sys.exit("every selected root must contain synthetic works")
    jobs = [(listener, (api, libs[i % len(libs)], stop, stats, i == 0)) for i in range(args.listeners)]
    jobs += [(video, (api, libs[0], stop, stats)), (latency_probe, (api, stop, stats))]
    before = process_sample(pid)

    def sample_process():
        sample = process_sample(pid)
        if sample["start_ticks"] != before["start_ticks"]:
            sys.exit("server PID was reused during the measurement")
        return sample

    # Separate streams keep thread scheduling from changing a seeded workload.
    workers = [threading.Thread(target=guarded, args=(job, stop, stats, *job_args), daemon=True,
                                kwargs={"seed": None if args.seed is None else args.seed + i})
               for i, (job, job_args) in enumerate(jobs)]
    for worker in workers:
        worker.start()

    start = time.monotonic()
    duration = args.hours * 3600
    samples, rescanned, throttled, blind = [], False, False, False
    latencies = []
    while time.monotonic() - start < duration:
        time.sleep(min(args.sample_seconds, max(0, duration - (time.monotonic() - start))))
        elapsed = time.monotonic() - start
        window = stats.take_latencies()
        latencies.extend(window)
        sample = {"t": round(elapsed), **sample_process(),
                  "cache_mib": round(cache_bytes(args.data_dir) / 2**20, 1),
                  "api_p95_ms": p95(window)}
        samples.append(sample)
        print(json.dumps(sample), file=sys.stderr, flush=True)
        if args.rescan_root and not rescanned and elapsed > duration / 2:
            status, body = api.request("POST", "/api/admin/scan", {"root": args.rescan_root})
            stats.event(f"rescan_{status}")
            for task_id in (body or {}).get("task_ids", []):
                stats.task(task_id)
            rescanned = True
        seen = check_throttle()
        if seen is None:
            blind = True
            break
        if seen:
            throttled = True
            break
    stop.set()
    load_seconds = time.monotonic() - start
    load_end = sample_process()
    if throttled or blind:
        # Server tasks outlive the workers, so stop them from reaching the
        # provider too.
        cancel_tasks(api, stats)
    for worker in workers:
        worker.join(timeout=60)
    latencies.extend(stats.take_latencies())
    cooldown = []
    cooling = time.monotonic()
    while time.monotonic() - cooling < args.cooldown_seconds:
        time.sleep(min(args.sample_seconds, max(0, args.cooldown_seconds - (time.monotonic() - cooling))))
        sample = {"idle_seconds": round(time.monotonic() - cooling), **sample_process()}
        cooldown.append(sample)
        print(json.dumps(sample), file=sys.stderr, flush=True)
    after = sample_process()

    steady = [s for s in samples if s["t"] >= args.warmup_minutes * 60]
    report = {
        "hours": round((time.monotonic() - start) / 3600, 2),
        "load_seconds": load_seconds,
        "cpu_seconds_per_load_hour": (load_end["cpu_seconds"] - before["cpu_seconds"]) * 3600 / load_seconds,
        "reaped_children_cpu_seconds_per_load_hour": (
            load_end["reaped_children_cpu_seconds"] - before["reaped_children_cpu_seconds"]
        ) * 3600 / load_seconds,
        "children_cpu_seconds_per_load_hour": (
            load_end["reaped_children_cpu_seconds"] + load_end["live_children_cpu_seconds"]
            - before["reaped_children_cpu_seconds"] - before["live_children_cpu_seconds"]
        ) * 3600 / load_seconds,
        "before": before,
        "load_end": load_end,
        "cooldown": cooldown,
        "api_latency_ms": {"p50": percentile(latencies, 0.50),
                           "p95": percentile(latencies, 0.95),
                           "p99": percentile(latencies, 0.99),
                           "samples": latencies},
        "stopped_on_throttle": throttled,
        "stopped_log_unreadable": blind,
        "status_codes": stats.codes,
        # 503 is the contract's answer while the storage is unreachable, so it
        # is counted apart from real server errors.
        "server_errors": sum(n for code, n in stats.codes.items() if code >= 500 and code != 503),
        "storage_unavailable": stats.codes.get(503, 0),
        "transport_errors": stats.codes.get(0, 0),
        "events": stats.events,
        "cache_budget": budget,
        "peak_cache_mib": max((s["cache_mib"] for s in samples), default=None),
        "rss_mib": {"first": steady[0]["rss_mib"] if steady else None,
                    "last": steady[-1]["rss_mib"] if steady else None,
                    "max": max((s["rss_mib"] for s in steady), default=None),
                    "slope_per_hour": slope_per_hour([(s["t"], s["rss_mib"]) for s in steady])},
        "fds": {"min": min((s["fds"] for s in steady), default=None),
                "max": max((s["fds"] for s in steady), default=None),
                "slope_per_hour": slope_per_hour([(s["t"], s["fds"]) for s in steady])},
        "threads_max": max((s["threads"] for s in samples), default=None),
        "ffmpeg_max": max((s["ffmpeg"] for s in samples), default=None),
        "api_p95_ms_max": max((s["api_p95_ms"] or 0 for s in samples), default=None),
        "after_stop": after,
        "samples": samples,
    }
    print(json.dumps(report, indent=1))


if __name__ == "__main__":
    main()
