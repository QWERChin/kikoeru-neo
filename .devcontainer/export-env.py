"""Export the prepared Nix environment without its temporary build identity."""

import json
from pathlib import Path
import shlex
import sys

source, script, bin_path = map(Path, sys.argv[1:])
variables = json.loads(source.read_text())["variables"]
# These describe the disposable environment-capture process, not a terminal.
excluded = {
    "HOME", "USER", "LOGNAME", "SHELL", "PWD", "OLDPWD", "SHLVL", "_",
    "TMP", "TMPDIR", "TEMP", "TEMPDIR", "NIX_BUILD_TOP", "NIX_ENFORCE_PURITY",
    "out", "outputs", "BASH_ENV", "ENV", "SSL_CERT_FILE", "NIX_SSL_CERT_FILE",
    "NIX_BUILD_CORES", "TERM", "TZ",
}
paths = [p for p in variables["PATH"]["value"].split(":") if p.startswith("/nix/store/")]
lines = ["# Prepared at image build time. No Nix command runs when a shell starts."]
for name, entry in variables.items():
    if entry["type"] == "exported" and name not in excluded and name != "PATH":
        lines.append(f"export {name}={shlex.quote(entry['value'])}")
lines.append("export PATH=" + shlex.quote(":".join(paths) + ":/nix/var/nix/profiles/default/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"))
lines.append("export SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt")
script.write_text("\n".join(lines) + "\n")

# docker exec bypasses the entrypoint. These dispatchers also supply the
# compiler's Nix flags when a tool is invoked without a shell.
bindir = bin_path
bindir.mkdir(parents=True)
for directory in paths:
    for executable in Path(directory).iterdir():
        target = bindir / executable.name
        if executable.is_file() and executable.stat().st_mode & 0o111 and not target.is_symlink():
            target.symlink_to("/usr/local/bin/dev-command")
