{ pkgs }:

pkgs.runCommand "kikoeru-neo-docker-smoke-check"
  { nativeBuildInputs = [ pkgs.bats pkgs.shellcheck ]; }
  ''
    shellcheck ${../scripts/docker-smoke.sh}
    SMOKE=${../scripts/docker-smoke.sh} bats ${../scripts/tests/docker-smoke.bats}
    touch $out
  ''
