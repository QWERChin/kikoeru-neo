{ pkgs }:

pkgs.runCommand "kikoeru-neo-release-ci-gate-check"
  { nativeBuildInputs = [ pkgs.python3 pkgs.git ]; }
  ''
    export HOME="$TMPDIR"
    mkdir -p scripts/tests
    cp ${../scripts/release-ci-gate.py} scripts/release-ci-gate.py
    cp ${../scripts/tests/release-ci-gate.py} scripts/tests/release-ci-gate.py
    python3 scripts/tests/release-ci-gate.py
    touch $out
  ''
