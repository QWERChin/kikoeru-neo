{ pkgs }:

pkgs.runCommand "kikoeru-neo-pnpm-audit-check"
  { nativeBuildInputs = [ pkgs.python3 ]; }
  ''
    export HOME="$TMPDIR"
    mkdir -p scripts/tests
    cp ${../scripts/pnpm-audit.py} scripts/pnpm-audit.py
    cp ${../scripts/pnpm-audit-exceptions.json} scripts/pnpm-audit-exceptions.json
    cp ${../scripts/tests/pnpm-audit.py} scripts/tests/pnpm-audit.py
    python3 scripts/tests/pnpm-audit.py
    touch $out
  ''
