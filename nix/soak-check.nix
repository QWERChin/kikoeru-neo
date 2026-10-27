{ pkgs }:

pkgs.runCommand "kikoeru-neo-soak-check"
  { nativeBuildInputs = [ pkgs.python3 ]; }
  ''
    mkdir -p scripts/tests
    cp ${../scripts/soak.py} scripts/soak.py
    cp ${../scripts/tests/soak.py} scripts/tests/soak.py
    python3 scripts/tests/soak.py
    touch $out
  ''
