# kikoeru-neo-docker-migrate, the host-side script that moves an old
# kikoeru container into a volume for the image, and its tests.
# docs/en/migration.md describes the container migration steps.
{ pkgs }:

rec {
  package = pkgs.writeShellApplication {
    name = "kikoeru-neo-docker-migrate";
    # docker or podman comes from the user's own PATH.
    # find removes the links in what docker cp brings out.
    runtimeInputs = [ pkgs.coreutils pkgs.findutils ];
    text = builtins.readFile ../scripts/docker-migrate.sh;
  };

  # The bats tests run the packaged script against a fake docker, so they
  # need no daemon. script(1) gives the terminal for the -t cases.
  check = pkgs.runCommand "kikoeru-neo-docker-migrate-check"
    {
      nativeBuildInputs = [ pkgs.bats pkgs.shellcheck pkgs.util-linux ];
    }
    ''
      shellcheck ${../scripts/docker-migrate.sh} ${../scripts/tests/fake-docker}
      # The sandbox has no /usr/bin/env for the fake docker's shebang.
      cp -r ${../scripts/tests} tests
      chmod -R u+w tests
      patchShebangs tests
      MIGRATE=${package}/bin/kikoeru-neo-docker-migrate MIGRATE_PACKAGED=1 bats tests/docker-migrate.bats
      touch $out
    '';
}
