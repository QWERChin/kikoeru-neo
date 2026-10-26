"""Start the standalone ELF without a wrapper, ffmpeg or installation data."""

from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request

with tempfile.TemporaryDirectory() as tmp:
    root = Path(tmp)
    config = root / "config.toml"
    config.write_text("")
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    env = {
        "HOME": tmp,
        "PATH": "/no-external-tools",
        "KIKOERU_DATA_DIR": str(root / "data"),
        "KIKOERU_CONFIG": str(config),
        "KIKOERU_LISTEN": f"127.0.0.1:{port}",
    }
    command = [sys.argv[1]]
    subprocess.run(
        [*command, "user", "create", "--admin", "admin", "--password-stdin"],
        input="synthetic-admin-password\n", text=True, env=env, check=True,
    )
    with (root / "server.log").open("w+") as log:
        server = subprocess.Popen([*command, "serve"], env=env, stdout=log, stderr=log)
        try:
            for _ in range(100):
                if server.poll() is not None:
                    raise RuntimeError("server exited before readiness")
                try:
                    with urllib.request.urlopen(f"http://127.0.0.1:{port}/api/ready", timeout=1) as r:
                        assert r.status == 200
                    with urllib.request.urlopen(f"http://127.0.0.1:{port}/", timeout=1) as r:
                        assert r.status == 200 and b"<html" in r.read()
                    break
                except (OSError, urllib.error.URLError):
                    time.sleep(0.1)
            else:
                raise RuntimeError("readiness timed out")
        finally:
            server.terminate()
            server.wait(timeout=10)
            log.seek(0)
            print(log.read())
