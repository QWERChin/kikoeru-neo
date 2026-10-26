{ pkgs }:

pkgs.runCommand "kikoeru-neo-github-release-check"
  { nativeBuildInputs = [ pkgs.python3 ]; }
  ''
    mkdir -p scripts/tests
    cp ${../scripts/github-release.py} scripts/github-release.py
    cp ${../scripts/tests/github-release.py} scripts/tests/github-release.py
    python3 scripts/tests/github-release.py
    touch $out
  ''
