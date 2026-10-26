{ pkgs, server }:

let rootfs = import ./docker-rootfs.nix { inherit pkgs server; };
in pkgs.runCommand "kikoeru-neo-docker-rootfs-check"
  { nativeBuildInputs = [ pkgs.python3 ]; }
  ''
    python3 - ${rootfs}/rootfs.tar <<'PY'
    import sys
    import tarfile

    with tarfile.open(sys.argv[1]) as archive:
        files = {m.name.removeprefix("./"): m for m in archive}
        for name in ["data", "backup"]:
            member = files[name]
            assert member.isdir()
            assert (member.uid, member.gid, member.mode) == (1000, 1000, 0o700)
            assert not any(p.startswith(name + "/") for p in files)
        assert files["tmp"].mode == 0o1777
        assert files["config"].isdir()
        for name in ["kikoeru-neo", "kikoeru-neo-healthcheck", "kikoeru-neo-entrypoint", "tini", "env", "chown"]:
            member = files["bin/" + name]
            assert member.issym(), name
            assert member.linkname.lstrip("/") in files, member.linkname
    PY
    touch $out
  ''
