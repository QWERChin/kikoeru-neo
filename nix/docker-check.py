"""Checks the image that nix/docker.nix streams, without a Docker daemon.

Usage: docker-check.py STREAM SERVER ENTRYPOINT HEALTHCHECK OUT

HEALTHCHECK is the script behind /bin/kikoeru-neo-healthcheck.

The expected values are written out here rather than read from docker.nix,
so a change to the image layout has to change this file too.
"""

import json
import pathlib
import subprocess
import sys
import tarfile
import zlib

stream, server, entrypoint, healthcheck, out = sys.argv[1:]
failures = []


def check(ok, message):
    if not ok:
        failures.append(message)


def normalize(name):
    # Store layers use absolute names, the customisation layer "./x".
    name = name.removeprefix("./").lstrip("/").rstrip("/")
    return name or "."


with open("image.tar", "wb") as f:
    subprocess.run([stream], stdout=f, check=True)

image = tarfile.open("image.tar")
manifest = json.load(image.extractfile("manifest.json"))
config = json.load(image.extractfile(manifest[0]["Config"]))["config"]

check(config.get("User") == "1000:1000", f"User is {config.get('User')!r}")
check(
    config.get("Cmd") == ["serve", "--config", "/config/config.toml"],
    f"Cmd is {config.get('Cmd')!r}",
)
entry = config.get("Entrypoint") or []
check(
    len(entry) == 3
    and entry[0].endswith("/bin/tini")
    and entry[1] == "--"
    and entry[2] == "/bin/kikoeru-neo-entrypoint",
    f"Entrypoint is {entry!r}",
)
shim = pathlib.Path(entrypoint).read_text()
check("umask 077\n" in shim, "the entrypoint does not set umask 077")
check(
    f'exec {server}/bin/kikoeru-neo "$@"' in shim,
    "the entrypoint does not exec the server",
)
env = sorted(config.get("Env") or [])
check(
    env
    == [
        "KIKOERU_DATA_DIR=/data",
        "KIKOERU_LISTEN=0.0.0.0:8888",
        "MALLOC_ARENA_MAX=2",
        "SQLITE_TMPDIR=/data/cache/tmp",
    ],
    f"Env is {env!r}",
)
check(
    config.get("Volumes") == {"/data": {}, "/backup": {}},
    f"Volumes is {config.get('Volumes')!r}",
)
check(
    (config.get("Labels") or {}).get("org.opencontainers.image.version")
    == pathlib.Path(server).name.rsplit("-", 1)[1],
    f"Labels is {config.get('Labels')!r}",
)
check(
    (config.get("Labels") or {}).get("org.opencontainers.image.source")
    == "https://github.com/QWERChin/kikoeru-neo",
    "the image is not linked to its repository",
)
check(
    (config.get("Labels") or {}).get("org.opencontainers.image.licenses")
    == "GPL-3.0-or-later",
    "the image license label is missing",
)
check(
    config.get("ExposedPorts") == {"8888/tcp": {}},
    f"ExposedPorts is {config.get('ExposedPorts')!r}",
)
health = config.get("Healthcheck") or {}
check(
    health.get("Test") == ["CMD", "/bin/kikoeru-neo-healthcheck"],
    f"Healthcheck.Test is {health.get('Test')!r}",
)
check(
    health.get("StartPeriod") == 120 * 10**9,
    f"Healthcheck.StartPeriod is {health.get('StartPeriod')!r}",
)
syntax = subprocess.run(["bash", "-n", healthcheck], capture_output=True, text=True)
check(syntax.returncode == 0, f"bash -n healthcheck: {syntax.stderr.strip()}")


def probe_address(listen):
    run = subprocess.run(
        [healthcheck, "--print-address"],
        env={"KIKOERU_LISTEN": listen},
        capture_output=True,
        text=True,
    )
    return run.stdout.strip() if run.returncode == 0 else None


# Each wildcard maps to the loopback of its own family, because an IPv6-only
# socket on [::] refuses 127.0.0.1.
for listen, address in [
    ("0.0.0.0:8888", "127.0.0.1 8888"),
    ("[::]:8888", "::1 8888"),
    ("127.0.0.1:9000", "127.0.0.1 9000"),
    ("192.0.2.7:80", "192.0.2.7 80"),
    ("[::1]:9000", "::1 9000"),
    ("[fd00::1:2]:8888", "fd00::1:2 8888"),
    ("", None),
]:
    got = probe_address(listen)
    check(got == address, f"healthcheck maps {listen!r} to {got!r}, not {address!r}")

layers = []
for name in manifest[0]["Layers"]:
    member = image.getmember(name)
    with tarfile.open(fileobj=image.extractfile(member)) as layer:
        entries = {normalize(ti.name): ti for ti in layer}
    layers.append((member.size, entries))


def find(path):
    found = [entries[path] for _, entries in layers if path in entries]
    return found[-1] if found else None


def under(path):
    return [p for _, entries in layers for p in entries if p.startswith(path + "/")]


for path, mode in [("data", 0o700), ("backup", 0o700)]:
    ti = find(path)
    check(ti is not None and ti.isdir(), f"/{path} is not a directory")
    if ti is not None:
        check(
            (ti.uid, ti.gid, ti.mode & 0o7777) == (1000, 1000, mode),
            f"/{path} is {ti.uid}:{ti.gid} {ti.mode & 0o7777:o}",
        )
for path in ["data", "backup"]:
    check(not under(path), f"/{path} is not empty: {under(path)[:5]}")
tmp = find("tmp")
check(tmp is not None and tmp.mode & 0o7777 == 0o1777, "/tmp is not 1777")
conf = find("config")
check(conf is not None and conf.isdir(), "/config is not a directory")
check(not under("config"), "/config is not empty")
for path in ["bin/kikoeru-neo", "bin/kikoeru-neo-healthcheck", "bin/env",
             "bin/tini", "bin/kikoeru-neo-entrypoint", "bin/chown"]:
    check(find(path) is not None, f"/{path} is missing")

# Its own layer, so a new build ships one changed layer.
server_path = normalize(server)
holding = [
    (size, entries) for size, entries in layers if server_path in entries
]
check(len(holding) == 1, f"neo-server is in {len(holding)} layers")
server_size = None
if len(holding) == 1:
    server_size, entries = holding[0]
    strays = [
        p
        for p in entries
        if p not in ("nix", "nix/store")
        and p != server_path
        and not p.startswith(server_path + "/")
    ]
    check(not strays, f"the neo-server layer also holds {strays[:5]}")

if failures:
    for message in failures:
        print(f"docker-image: {message}", file=sys.stderr)
    sys.exit(1)

total = sum(size for size, _ in layers)
gzipped = 0
compressor = zlib.compressobj(6, zlib.DEFLATED, 31)
with open("image.tar", "rb") as f:
    while chunk := f.read(1 << 20):
        gzipped += len(compressor.compress(chunk))
gzipped += len(compressor.flush())

mib = 1 << 20
report = (
    f"layers: {len(layers)}\n"
    f"neo-server layer: {server_size} bytes ({server_size / mib:.1f} MiB)\n"
    f"all layers: {total} bytes ({total / mib:.1f} MiB)\n"
    f"image tar, gzip -6: {gzipped} bytes ({gzipped / mib:.1f} MiB)\n"
)
print(report, end="")
out_dir = pathlib.Path(out)
out_dir.mkdir()
(out_dir / "sizes.txt").write_text(report)
