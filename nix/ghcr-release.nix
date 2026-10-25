# kikoeru-neo-ghcr-release, which the release workflow runs to push the
# image to GitHub Container Registry, and its tests.
{ pkgs }:

rec {
  package = pkgs.writeShellApplication {
    name = "kikoeru-neo-ghcr-release";
    runtimeInputs = [ pkgs.coreutils pkgs.skopeo ];
    # The policy that ships with this skopeo accepts any image, which is
    # all a push of our own build needs, and pins it with the flake.
    text = ''
      export KIKOERU_RELEASE_POLICY=''${KIKOERU_RELEASE_POLICY:-${pkgs.skopeo}/etc/containers/default-policy.json}
    '' + builtins.readFile ../scripts/ghcr-release.sh;
  };

  # The bats tests run the packaged script against a fake skopeo, so they
  # push nothing.
  check = pkgs.runCommand "kikoeru-neo-ghcr-release-check"
    {
      nativeBuildInputs = [ pkgs.bats pkgs.shellcheck ];
    }
    ''
      shellcheck ${../scripts/ghcr-release.sh} ${../scripts/tests/fake-skopeo}
      # The sandbox has no /usr/bin/env for the fake skopeo's shebang.
      cp -r ${../scripts/tests} tests
      chmod -R u+w tests
      patchShebangs tests
      RELEASE=${package}/bin/kikoeru-neo-ghcr-release RELEASE_PACKAGED=1 bats tests/ghcr-release.bats
      touch $out
    '';
}
