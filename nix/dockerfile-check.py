"""Compare the final Dockerfile stage to the image's shared Nix config.

Accept only the literal instruction forms used here, so substitutions or a
new instruction cannot silently escape the comparison.
"""

import json
from pathlib import Path
import re
import shlex
import sys


def parse(source):
    lines = re.sub(r"\\\n\s*", " ", source).splitlines()
    config = {}
    for line in lines:
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        instruction, value = line.split(None, 1)
        instruction = instruction.upper()
        if instruction == "FROM":
            config = {}
            final = value == "scratch"
            continue
        if not final:
            continue
        if instruction == "COPY":
            assert value == "--from=builder /runtime/ /", value
        elif instruction in ("ENTRYPOINT", "CMD"):
            config[{"ENTRYPOINT": "Entrypoint", "CMD": "Cmd"}[instruction]] = json.loads(value)
        elif instruction == "USER":
            config["User"] = value
        elif instruction == "ENV":
            config.setdefault("Env", []).extend(shlex.split(value))
        elif instruction == "LABEL":
            config.setdefault("Labels", {}).update(v.split("=", 1) for v in shlex.split(value))
        elif instruction == "EXPOSE":
            config.setdefault("ExposedPorts", {}).update((v, {}) for v in value.split())
        elif instruction == "VOLUME":
            config.setdefault("Volumes", {}).update((v, {}) for v in json.loads(value))
        elif instruction == "HEALTHCHECK":
            options, command = value.split(" CMD ", 1)
            health = {"Test": ["CMD", *json.loads(command)]}
            names = {"interval": "Interval", "timeout": "Timeout", "start-period": "StartPeriod",
                     "start-interval": "StartInterval", "retries": "Retries"}
            for option in options.split():
                name, val = option.removeprefix("--").split("=")
                if name == "retries":
                    health[names[name]] = int(val)
                else:
                    assert re.fullmatch(r"[0-9]+s", val), val
                    health[names[name]] = int(val[:-1]) * 10**9
            config["Healthcheck"] = health
        else:
            raise AssertionError(f"Unchecked final-stage instruction: {instruction}")
    assert final, "The final stage must be FROM scratch"
    config["Env"].sort()
    return config


source = Path(sys.argv[1]).read_text()
expected = json.loads(Path(sys.argv[2]).read_text())
expected["Env"].sort()
actual = parse(source)
assert actual == expected, f"Dockerfile config drift:\n{actual!r}\nExpected:\n{expected!r}"
# Exercise each compared field, including the newer start-interval option.
for before, after in [
    ('"/bin/tini"', '"/bin/false"'), ('"serve"', '"version"'),
    ('USER 1000:1000', 'USER 0:0'), ('MALLOC_ARENA_MAX=2', 'MALLOC_ARENA_MAX=3'),
    ('8888/tcp', '9999/tcp'), ('"/backup"', '"/backups"'),
    ('--start-interval=2s', '--start-interval=3s'),
    ('GPL-3.0-or-later', 'MIT'),
]:
    assert before in source
    assert parse(source.replace(before, after)) != expected, before
