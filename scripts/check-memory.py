#!/usr/bin/env python3
"""Rebuilds one flake check and prints its peak memory (book/src/testing.md,
"CI 的内存预算").

Usage: scripts/check-memory.py <check> [<check> ...]

The check must have been built before, because `nix build --rebuild` reruns
only the check itself, not what it depends on. Every 0.5 s the RSS of all
processes of each Nix build user is summed. Shared pages count once per
process, so the peak is an upper bound. Run nothing else on the machine's
Nix daemon meanwhile, or another build's processes may land in the same
build user.
"""

import subprocess
import sys
import threading
import time
from pathlib import Path

INTERVAL = 0.5


def build_users():
    users = {}
    for line in Path("/etc/passwd").read_text().splitlines():
        name, _, uid, *_ = line.split(":")
        if name.startswith("nixbld"):
            users[int(uid)] = name
    return users


def rss_by_user(users):
    total = {}
    for proc in Path("/proc").iterdir():
        if not proc.name.isdigit():
            continue
        try:
            fields = dict(
                line.split(":", 1) for line in (proc / "status").read_text().splitlines()
            )
        except (OSError, ValueError):
            continue
        uid = int(fields.get("Uid", "-1").split()[0])
        rss = fields.get("VmRSS")
        if uid in users and rss:
            total[uid] = total.get(uid, 0) + int(rss.split()[0])
    return total


def measure(check, users):
    peak = {}
    done = threading.Event()

    def sample():
        while not done.is_set():
            for uid, kib in rss_by_user(users).items():
                peak[uid] = max(peak.get(uid, 0), kib)
            time.sleep(INTERVAL)

    # A daemon thread, and stopped in `finally`, so Ctrl-C or a failure to
    # start nix never leaves the script running.
    sampler = threading.Thread(target=sample, daemon=True)
    sampler.start()
    start = time.monotonic()
    try:
        build = subprocess.run(
            ["nix", "build", "--rebuild", "--no-link", "--cores", "2",
             f".#checks.x86_64-linux.{check}"],
            capture_output=True, text=True,
        )
    finally:
        done.set()
        sampler.join()
    seconds = time.monotonic() - start
    # A rebuild whose output differs still ran the whole check, so only a
    # failure for any other reason is reported as one.
    failed = build.returncode != 0 and "may not be deterministic" not in build.stderr
    top = max(peak.values(), default=0)
    note = "  (the build failed: " + build.stderr.strip().splitlines()[-1] + ")" if failed else ""
    print(f"{check}: {top // 1024} MiB peak, {seconds:.0f} s{note}")


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    users = build_users()
    for check in sys.argv[1:]:
        measure(check, users)


if __name__ == "__main__":
    main()
