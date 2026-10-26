# Exercise the ordinary-distribution unit verbatim, without the NixOS module
# or the server package's PATH wrapper.
{ pkgs, server, ffmpeg }:

pkgs.testers.runNixOSTest {
  name = "kikoeru-neo-systemd";
  nodes.machine = {
    # Keep the default free-space reserve enabled during actual media work.
    virtualisation.diskSize = 8192;
    # Nixpkgs replaces systemd's compiled-in PATH with its own store path.
    # Model the upstream system manager default without changing the unit.
    systemd.settings.Manager.DefaultEnvironment = "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin";
    users.users.kikoeru-neo = {
      isSystemUser = true;
      group = "kikoeru-neo";
      home = "/var/lib/kikoeru-neo";
    };
    users.groups.kikoeru-neo = { };
    environment.systemPackages = [ pkgs.curl pkgs.python3 ];
    environment.etc."kikoeru-neo/config.toml".text = ''
      listen = "127.0.0.1:8888"
      [metadata]
      source = "none"
      [[roots]]
      type = "local"
      id = "local"
      display_name = "Synthetic audio"
      path = "/srv/voice"
    '';
  };
  testScript = ''
    import json
    import shlex

    machine.start()
    machine.wait_for_unit("multi-user.target")
    machine.succeed("install -D -m 755 ${server}/bin/kikoeru-neo /usr/local/bin/kikoeru-neo")
    # /etc/systemd/system is a read-only store link on NixOS. The runtime
    # unit directory accepts the same file without generating a NixOS unit.
    machine.succeed("install -D -m 644 ${../packaging/systemd/kikoeru-neo.service} /run/systemd/system/kikoeru-neo.service")
    machine.succeed("cmp ${../packaging/systemd/kikoeru-neo.service} /run/systemd/system/kikoeru-neo.service")
    # Do not add ffmpeg to systemPackages: only /usr/bin may resolve it.
    machine.succeed("mkdir -p /usr/bin /srv/voice/RJ000001")
    machine.succeed("ln -s ${ffmpeg}/bin/ffmpeg /usr/bin/ffmpeg")
    machine.succeed("ln -s ${ffmpeg}/bin/ffprobe /usr/bin/ffprobe")
    machine.succeed("/usr/bin/ffmpeg -nostdin -v error -f lavfi -i sine=duration=1 /srv/voice/RJ000001/tone.wav")
    machine.succeed("chmod -R a+rX /srv/voice")
    machine.succeed("systemd-analyze verify /run/systemd/system/kikoeru-neo.service")
    machine.succeed("systemctl daemon-reload")
    machine.succeed("systemctl enable --runtime --now kikoeru-neo.service")
    machine.wait_for_unit("kikoeru-neo.service")
    # Type=simple is active before first-start migrations finish. Management
    # commands must wait for readiness before opening that same database.
    machine.wait_until_succeeds("test $(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8888/api/ready) = 200")

    # These are the guide's management commands, with a synthetic password on
    # stdin instead of an interactive password prompt. env -i drops any caller
    # database override; the shell applies the service's umask before exec.
    as_service = (
        "runuser -u kikoeru-neo -- env -i"
        " PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
        " HOME=/var/lib/kikoeru-neo"
        " KIKOERU_DATA_DIR=/var/lib/kikoeru-neo"
        " KIKOERU_CONFIG=/etc/kikoeru-neo/config.toml"
        " /bin/sh -c 'umask 077; exec /usr/local/bin/kikoeru-neo \"$@\"' sh"
    )
    machine.succeed(f"printf 'synthetic password\\n' | {as_service} user create --admin admin --password-stdin")
    machine.wait_until_succeeds("test $(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8888/api/ready) = 200")
    assert machine.succeed("stat -c '%U:%G %a' /var/lib/kikoeru-neo").strip() == "kikoeru-neo:kikoeru-neo 700"
    assert machine.succeed("stat -c '%U:%G %a' /var/lib/kikoeru-neo/neo.sqlite3").strip() == "kikoeru-neo:kikoeru-neo 600"
    pid = machine.succeed("systemctl show -p MainPID --value kikoeru-neo").strip()
    machine.succeed(f"grep -q '^Umask:[[:space:]]*0077$' /proc/{pid}/status")
    path = machine.succeed(f"tr '\\0' '\\n' < /proc/{pid}/environ | sed -n 's/^PATH=//p'").strip()
    assert "/usr/bin" in path.split(":"), path
    machine.succeed(f"env PATH={shlex.quote(path)} ${pkgs.bash}/bin/bash -c 'test \"$(command -v ffmpeg)\" = /usr/bin/ffmpeg'")

    def request(method, route, body=None, token=None):
        command = f"curl -sf -X {method} http://127.0.0.1:8888/api/{route}"
        if token:
            command += " -H " + shlex.quote(f"authorization: Bearer {token}")
        if body is not None:
            command += " -H 'content-type: application/json' -d " + shlex.quote(json.dumps(body))
        return command

    login = json.loads(machine.succeed(request("POST", "auth/login", {"name": "admin", "password": "synthetic password"})))
    assert login["user"]["is_admin"]
    token = login["token"]

    def api(method, route, body=None):
        return json.loads(machine.succeed(request(method, route, body, token)))

    def wait_task(task_id):
        command = request("GET", f"tasks/{task_id}", token=token)
        check = "import json,sys; assert json.load(sys.stdin)['state'] in ['done','failed','cancelled','interrupted']"
        machine.wait_until_succeeds(command + " | python3 -c " + shlex.quote(check))
        task = api("GET", f"tasks/{task_id}")
        assert task["state"] == "done", task

    for task in api("POST", "admin/scan", {"root": "local"})["task_ids"]:
        wait_task(task)
    work = api("GET", "works")["items"][0]
    track = next(t for t in api("GET", f"works/{work['id']}")["tracks"] if t["kind"] == "audio")
    # Scanning does not analyze audio. This request must run ffmpeg inside the
    # unit's sandbox, not merely list its advertised capabilities at startup.
    result = api("GET", f"tracks/{track['id']}/loudness")
    wait_task(result["task_id"])
    loudness = api("GET", f"tracks/{track['id']}/loudness")
    assert -70 < loudness["integrated_lufs"] < 0, loudness
    machine.succeed(request("GET", f"tracks/{track['id']}/waveform", token=token) + " -o /tmp/waveform")
    assert machine.succeed("stat -c %s /tmp/waveform").strip() == "1000"
    machine.fail("journalctl -u kikoeru-neo -o cat | grep -F 'media tool check:'")

    machine.succeed("systemctl stop kikoeru-neo.service")
    machine.succeed("install -d -o kikoeru-neo -g kikoeru-neo -m 700 /var/backups/kikoeru-neo")
    machine.succeed(f"{as_service} backup /var/backups/kikoeru-neo/snapshot")
    for file in ["neo.sqlite3", "secrets/jwt.key"]:
        machine.succeed(f"test -s /var/backups/kikoeru-neo/snapshot/{file}")
        assert machine.succeed(f"stat -c '%U:%G %a' /var/backups/kikoeru-neo/snapshot/{file}").strip() == "kikoeru-neo:kikoeru-neo 600"
    assert machine.succeed("stat -c %a /var/backups/kikoeru-neo/snapshot").strip() == "700"
    backup_check = "import sqlite3; db = sqlite3.connect('file:/var/backups/kikoeru-neo/snapshot/neo.sqlite3?mode=ro', uri=True); assert db.execute('SELECT name FROM users').fetchall() == [('admin',)]"
    machine.succeed("python3 -c " + shlex.quote(backup_check))

    machine.succeed("rm /usr/bin/ffmpeg")
    machine.succeed("systemctl start kikoeru-neo.service")
    machine.wait_until_succeeds("test $(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8888/api/ready) = 200")
    warning = 'media tool check: ffmpeg "ffmpeg": cannot run -version (No such file or directory (os error 2)); affected features: audio transcoding, HLS video, loudness/waveform analysis; install a complete ffmpeg build or set media.ffmpeg in config.toml'
    machine.wait_until_succeeds("journalctl -u kikoeru-neo -p warning -o cat | grep -F " + shlex.quote(warning))
    logs = machine.succeed("journalctl -u kikoeru-neo -p warning -o cat")
    assert logs.count(warning) == 1, logs
    machine.succeed(request("POST", "auth/login", {"name": "admin", "password": "synthetic password"}))
  '';
}
