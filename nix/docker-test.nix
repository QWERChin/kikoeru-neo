# Migrates synthetic old containers inside a VM using docs/en/migration.md,
# then runs the new image as in docs/en/install-docker.md. Source containers
# are made with `docker create` and never started during migration.
{ pkgs, image, migrate }:

let
  ref = "${image.imageName}:${image.imageTag}";

  schema = ../crates/neo-server/tests/fixtures/legacy_0614.sql;

  # An old 0.6.14 data directory: the synthetic fixture, a config.json with
  # one media library and secrets that must never come over, one cover and
  # one track.
  oldData = pkgs.runCommand "kikoeru-old-data" { nativeBuildInputs = [ pkgs.sqlite ]; } ''
    mkdir -p $out/sqlite $out/config $out/covers $out/VoiceWork/RJ123456
    sqlite3 $out/sqlite/db.sqlite3 < ${schema}
    cat > $out/config/config.json <<'EOF'
    {
      "version": "0.6.14",
      "sqliteType": "sqlite3",
      "dbUseDefaultPath": true,
      "coverUseDefaultPath": true,
      "jwtsecret": "old-jwt-secret",
      "md5secret": "old-md5-secret",
      "rootFolders": [{ "name": "Old", "path": "/usr/src/kikoeru/VoiceWork" }]
    }
    EOF
    printf 'cover' > $out/covers/RJ123456_img_main.jpg
    printf 'audio' > $out/VoiceWork/RJ123456/01.mp3
  '';

  # A database with a third work, for an old install whose config.json
  # moves the database out of sqlite/.
  movedDb = pkgs.runCommand "kikoeru-old-moved-db" { nativeBuildInputs = [ pkgs.sqlite ]; } ''
    mkdir -p $out
    {
      cat ${schema}
      echo "INSERT INTO t_work (id, root_folder, dir, title, circle_id, memo)"
      echo "  VALUES (345678, 'Old', 'RJ345678', 'Third', 11, '{}');"
    } | sqlite3 $out/db.sqlite3
  '';
  movedConfig = pkgs.writeText "config.json" (builtins.toJSON {
    version = "0.6.14";
    sqliteType = "sqlite3";
    dbUseDefaultPath = false;
    databaseFolderDir = "db";
    coverUseDefaultPath = true;
    rootFolders = [{ name = "Old"; path = "/usr/src/kikoeru/VoiceWork"; }];
  });

  sharedConfig = pkgs.writeText "config.json" (builtins.toJSON {
    version = "0.6.14";
    sqliteType = "sqlite3";
    dbUseDefaultPath = false;
    databaseFolderDir = "/legacy-store";
    coverUseDefaultPath = false;
    coverFolderDir = "/legacy-store";
    rootFolders = [{ name = "Old"; path = "/usr/src/kikoeru/VoiceWork"; }];
  });

  # The layout of the official compose file, but with the data directory
  # at /data and a relative media path, as an old config.json may have.
  splitConfig = pkgs.writeText "config.json" (builtins.toJSON {
    version = "0.6.14";
    sqliteType = "sqlite3";
    dbUseDefaultPath = true;
    coverUseDefaultPath = true;
    rootFolders = [{ name = "Old"; path = "VoiceWork"; }];
  });

  # Enough works and covers that a stop lands in the middle of the import.
  bigData = pkgs.runCommand "kikoeru-old-big" { nativeBuildInputs = [ pkgs.sqlite pkgs.python3 ]; } ''
    mkdir -p $out/sqlite $out/covers
    {
      cat ${schema}
      echo "WITH RECURSIVE n(i) AS (SELECT 300000 UNION ALL SELECT i + 1 FROM n WHERE i < 359999)"
      echo "INSERT INTO t_work (id, root_folder, dir, title, circle_id, memo)"
      echo "  SELECT i, 'Old', 'RJ' || i, 'Work ' || i, 11, '{}' FROM n;"
    } | sqlite3 $out/sqlite/db.sqlite3
    python3 -c '
    import sys
    for i in range(300000, 360000):
        with open(f"{sys.argv[1]}/RJ{i}_img_main.jpg", "wb") as f:
            f.write(b"c" * 2048)
    ' $out/covers
  '';

  # The old image only has to exist. Its data sits in the image layer for
  # the container that has no volumes.
  oldImage = pkgs.dockerTools.buildImage {
    name = "fake-kikoeru";
    tag = "old";
    copyToRoot = pkgs.runCommand "fake-kikoeru-root" { } ''
      mkdir -p $out/usr/src
      cp -r ${oldData} $out/usr/src/kikoeru
    '';
    config = {
      WorkingDir = "/usr/src/kikoeru";
      Cmd = [ "/the-old-container-must-not-start" ];
    };
  };
in
pkgs.testers.runNixOSTest {
  name = "kikoeru-neo-docker";
  # Bound hangs in Docker or the guest to 20 minutes.
  globalTimeout = 1200;
  nodes.machine = {
    virtualisation.docker.enable = true;
    virtualisation.diskSize = 4096;
    virtualisation.memorySize = 2048;
    virtualisation.additionalPaths = [ image oldImage oldData bigData movedDb movedConfig sharedConfig splitConfig ];
    environment.systemPackages = [ migrate pkgs.curl pkgs.jq ];
  };
  testScript = ''
    import threading
    import time

    # Host-wide pressure stall and free memory every minute, so a hang or
    # a slow step in the build log shows whether the CI host was starved.
    def pressure():
        lines = []
        for resource in ("cpu", "memory", "io"):
            try:
                with open(f"/proc/pressure/{resource}") as f:
                    lines += [f"{resource} {line}" for line in f.read().splitlines()]
            except OSError:
                pass
        with open("/proc/meminfo") as f:
            lines += [l.strip() for l in f if l.startswith(("MemAvailable:", "SwapFree:"))]
        return "\n".join(lines)

    def sample_pressure():
        while True:
            print(f"pressure at {time.strftime('%H:%M:%S', time.gmtime())}:\n{pressure()}", flush=True)
            time.sleep(60)

    threading.Thread(target=sample_pressure, daemon=True).start()

    def fails_with(command, text):
        status, output = machine.execute(command + " 2>&1")
        assert status != 0, f"{command} succeeded:\n{output}"
        assert text in output, f"{command} did not say {text!r}:\n{output}"

    def wait_healthy(name):
        machine.wait_until_succeeds(
            f"test \"$(docker inspect -f '{{{{.State.Health.Status}}}}' {name})\" = healthy",
            timeout=180,
        )

    def login(password):
        return machine.succeed(
            "curl -sf -X POST -H 'content-type: application/json'"
            f" -d '{{\"name\":\"admin\",\"password\":\"{password}\"}}'"
            " http://127.0.0.1:8888/api/auth/login | jq -er .token"
        ).strip()

    def volume_dir(name):
        return machine.succeed(f"docker volume inspect -f '{{{{.Mountpoint}}}}' {name}").strip()

    machine.wait_for_unit("docker.service")
    machine.succeed("${image} | docker load")
    machine.succeed("docker load < ${oldImage}")

    # 1. Old containers that never ran: one whose data is a bind mount that
    # uid 1000 can read, one with the data in its own layer, and one with
    # root-owned 0600 data on a bind mount.
    machine.succeed("cp -r ${oldData} /srv/old && chmod -R u+w,a+rX /srv/old")
    machine.succeed(
        "cp -r ${oldData} /srv/private && chmod -R u+w,go-rwx /srv/private"
    )
    machine.succeed("docker create --name old -v /srv/old:/usr/src/kikoeru fake-kikoeru:old")
    machine.succeed("docker create --name old-layer fake-kikoeru:old")
    machine.succeed("docker create --name old-private -v /srv/private:/usr/src/kikoeru fake-kikoeru:old")

    # 2. The script without a terminal: machine.succeed has none.
    out = machine.succeed(
        "cd /root && kikoeru-neo-docker-migrate --from-container old"
        " --image ${ref} --out /root/neo --root Old=voice"
    )
    assert "copying the old database and covers" in out, out
    # The compose snippet mounts just the library, not the old data folder.
    assert "- /srv/old/VoiceWork:/usr/src/kikoeru/VoiceWork:ro" in out, out
    assert "- /srv/old:" not in out, out
    config = machine.succeed("cat /root/neo/config.toml")
    assert 'id = "voice"' in config and "/usr/src/kikoeru/VoiceWork" in config, config
    assert "old-jwt-secret" not in config and "old-md5-secret" not in config, config
    assert machine.succeed("stat -c %a /root/neo/config.toml").strip() == "644"
    data = volume_dir("kikoeru-neo-data")
    machine.succeed(f"test -s {data}/neo.sqlite3 && test ! -e {data}/config.toml")
    assert machine.succeed(f"stat -c '%u %a' {data}").strip() == "1000 700"

    # The same volume again is refused and left as it is.
    fails_with(
        "cd /root && kikoeru-neo-docker-migrate --from-container old --image ${ref} --out /root/neo2",
        "already exists",
    )
    machine.succeed(f"test -s {data}/neo.sqlite3")

    # The two containers whose data uid 1000 cannot read are copied out.
    for name in ["old-layer", "old-private"]:
        out = machine.succeed(
            f"mkdir -p /root/stage-{name} && cd /root && kikoeru-neo-docker-migrate"
            f" --from-container {name} --image ${ref} --volume neo-{name}"
            f" --out /root/neo-{name} --stage-dir /root/stage-{name}"
        )
        assert "copying the old database and covers" in out, out
        machine.succeed(f"test -s {volume_dir('neo-' + name)}/neo.sqlite3")
        machine.succeed(f"test -z \"$(ls -A /root/stage-{name})\"")

    # Only the moved database folder is a mount. config.json, which names
    # that folder, and covers/ are in the container's own layer, where
    # docker cp writes the config before anything reads it.
    machine.succeed("cp -r ${movedDb} /srv/moved-db && chmod -R u+w,a+rX /srv/moved-db")
    machine.succeed("docker create --name old-moved -v /srv/moved-db:/usr/src/kikoeru/db fake-kikoeru:old")
    machine.succeed("docker cp ${movedConfig} old-moved:/usr/src/kikoeru/config/config.json")
    machine.succeed(
        "mkdir -p /root/stage-moved && cd /root && kikoeru-neo-docker-migrate --from-container old-moved"
        " --image ${ref} --volume neo-moved --out /root/neo-moved --stage-dir /root/stage-moved"
    )
    moved = volume_dir("neo-moved")
    assert machine.succeed(f"jq '.work_ids | length' {moved}/import-report.json").strip() == "3"
    assert machine.succeed(f"ls {moved}/covers | wc -l").strip() == "1"

    # The custom database and cover directories may be the same. Both
    # inputs must remain visible through a single read-only staged mount.
    machine.succeed(
        "mkdir -p /srv/shared"
        " && cp ${movedDb}/db.sqlite3 ${oldData}/covers/RJ123456_img_main.jpg /srv/shared/"
        " && chmod -R u+rwX,go-rwx /srv/shared"
        " && cp -r /srv/shared /srv/shared.before"
    )
    machine.succeed("docker create --name old-shared -v /srv/shared:/legacy-store fake-kikoeru:old")
    machine.succeed("docker cp ${sharedConfig} old-shared:/usr/src/kikoeru/config/config.json")
    machine.succeed(
        "mkdir -p /root/stage-shared && cd /root && kikoeru-neo-docker-migrate --from-container old-shared"
        " --image ${ref} --volume neo-shared --out /root/neo-shared --stage-dir /root/stage-shared"
    )
    shared = volume_dir("neo-shared")
    machine.succeed(f"test -s {shared}/neo.sqlite3")
    assert machine.succeed(f"jq '.work_ids | length' {shared}/import-report.json").strip() == "3"
    machine.succeed(f"test \"$(find {shared}/covers -type f | wc -l)\" -eq 1")
    machine.succeed(f"cmp /srv/shared/RJ123456_img_main.jpg {shared}/covers/*")
    machine.succeed("test -z \"$(ls -A /root/stage-shared)\"")

    # KIKOERU_DATA_DIR=/data with sqlite/, config/, covers/ and the media
    # mounted separately. --volumes-from brings those mounts under /data,
    # so the new library must be written elsewhere, and the relative media
    # path must resolve against the old /data, where the media mount is.
    machine.succeed(
        "cp -r ${oldData} /srv/split && chmod -R u+w,a+rX /srv/split"
        " && install -m 644 ${splitConfig} /srv/split/config/config.json"
    )
    machine.succeed("cp -r /srv/split /srv/split.before")
    machine.succeed(
        "docker create --name old-split -e KIKOERU_DATA_DIR=/data"
        + "".join(f" -v /srv/split/{d}:/data/{d}" for d in ["sqlite", "config", "covers", "VoiceWork"])
        + " fake-kikoeru:old"
    )
    out = machine.succeed(
        "mkdir -p /root/stage-split && cd /root && kikoeru-neo-docker-migrate --from-container old-split"
        " --image ${ref} --volume neo-split --out /root/neo-split --stage-dir /root/stage-split"
    )
    split = volume_dir("neo-split")
    machine.succeed(f"test -s {split}/neo.sqlite3")
    for d in ["sqlite", "config", "VoiceWork"]:
        machine.succeed(f"test ! -e {split}/{d}")
    config = machine.succeed("cat /root/neo-split/config.toml")
    assert '\npath = "/data/VoiceWork"' in config, config
    assert "/data/VoiceWork is inside /data" in out, out

    # config.json and the database are mounted as single root-owned 0600
    # files. --volumes-from stacks them over the staged copies unless the
    # copies are bound at those exact paths.
    machine.succeed(
        "install -d -m 0700 /srv/files"
        " && install -m 0600 ${oldData}/config/config.json /srv/files/config.json"
        " && install -m 0600 ${oldData}/sqlite/db.sqlite3 /srv/files/db.sqlite3"
    )
    machine.succeed(
        "docker create --name old-files"
        " -v /srv/files/config.json:/usr/src/kikoeru/config/config.json"
        " -v /srv/files/db.sqlite3:/usr/src/kikoeru/sqlite/db.sqlite3 fake-kikoeru:old"
    )
    machine.succeed(
        "mkdir -p /root/stage-files && cd /root && kikoeru-neo-docker-migrate --from-container old-files"
        " --image ${ref} --volume neo-files --out /root/neo-files --stage-dir /root/stage-files"
    )
    machine.succeed(f"test -s {volume_dir('neo-files')}/neo.sqlite3")

    # A running old container is refused. The fake old image has no
    # programs, so the stand-in runs coreutils' sleep from the new image.
    machine.succeed("docker run -d --name running-old --entrypoint /bin/sleep ${ref} 600")
    fails_with(
        "kikoeru-neo-docker-migrate --from-container running-old --image ${ref} --volume never --out /root/never",
        "is running",
    )
    machine.fail("docker volume inspect never")
    machine.succeed("docker rm -f running-old")

    # 3. serve with a read-only root, as in the compose example. It turns
    # healthy, and the host reaches it, which needs KIKOERU_LISTEN.
    serve = (
        "docker run -d --name neo --read-only --tmpfs /tmp --cap-drop ALL"
        " --security-opt no-new-privileges:true -p 127.0.0.1:8888:8888"
        " -v kikoeru-neo-data:/data -v /root/neo/config.toml:/config/config.toml:ro"
        " -v /srv/old/VoiceWork:/usr/src/kikoeru/VoiceWork:ro ${ref}"
    )
    machine.succeed(serve)
    wait_healthy("neo")
    machine.succeed("curl -sf http://127.0.0.1:8888/api/ready")

    # 4. Imported users have no password until one is set.
    machine.succeed(
        "echo 'new admin password' | docker exec -i neo kikoeru-neo user set-password admin --password-stdin"
    )
    token = login("new admin password")
    machine.succeed(
        f"curl -sf -H 'authorization: Bearer {token}' 'http://127.0.0.1:8888/api/works?page=1'"
        " | jq -e '.items | type == \"array\"'"
    )

    # 5. The signing key is in the volume, so a token outlives a restart.
    machine.succeed("docker restart neo")
    wait_healthy("neo")
    machine.succeed(f"curl -sf -H 'authorization: Bearer {token}' http://127.0.0.1:8888/api/auth/me")
    machine.succeed("timeout 30 docker stop neo")
    assert machine.succeed("docker inspect -f '{{.State.ExitCode}}' neo").strip() == "0"
    machine.succeed("docker rm neo")

    # 6. Data directories the server must refuse, and a missing config.
    machine.succeed("install -d -m 0777 /srv/open && install -d -m 0700 -o root /srv/rootowned")
    run = "docker run --rm -v /root/neo/config.toml:/config/config.toml:ro"
    fails_with(f"{run} -v /srv/open:/data ${ref}", "writable by group or others")
    fails_with(f"{run} -v /srv/rootowned:/data ${ref}", "Permission denied")
    fails_with("docker run --rm -v kikoeru-neo-data:/data ${ref}", "cannot read config file /config/config.toml")

    # 7. docker stop in the middle of an import: tini forwards SIGTERM, the
    # import cancels and cleans up, and no .import-lock is left behind.
    machine.succeed("docker create --name old-big -v ${bigData}:/usr/src/kikoeru:ro fake-kikoeru:old")
    machine.succeed("docker volume create neo-big")
    big = volume_dir("neo-big")
    machine.succeed(
        "docker run -d --name big --volumes-from old-big:ro -v neo-big:/data ${ref}"
        " migrate --source /usr/src/kikoeru --root Old=old --yes"
    )
    machine.succeed(
        f"for i in $(seq 1 6000); do test -e {big}/.import-lock && exit 0; sleep 0.01; done; exit 1"
    )
    machine.succeed("timeout 30 docker stop big")
    logs = machine.succeed("docker logs big 2>&1")
    # The cancel landed while rows were being written, not before or after.
    assert "the import was cancelled" in logs, logs
    machine.succeed(f"test ! -e {big}/.import-lock && test ! -e {big}/neo.sqlite3")

    # 8. Nothing of the old data changed, and no SQLite side files appeared.
    machine.succeed("diff -r ${oldData} /srv/old")
    machine.succeed("diff -r ${oldData} /srv/private")
    machine.succeed("diff -r ${movedDb} /srv/moved-db")
    machine.succeed("diff -r /srv/shared.before /srv/shared")
    machine.succeed("diff -r /srv/split.before /srv/split")
    machine.succeed("docker cp old-layer:/usr/src/kikoeru /root/layer-after && diff -r ${oldData} /root/layer-after")
    # The old containers are still the never-started ones.
    for name in ["old", "old-layer", "old-private", "old-moved", "old-shared", "old-split", "old-files", "old-big"]:
        assert machine.succeed(
            f"docker inspect -f '{{{{.State.Status}}}} {{{{.State.StartedAt}}}}' {name}"
        ).strip().startswith("created 0001-01-01"), name
  '';
}
