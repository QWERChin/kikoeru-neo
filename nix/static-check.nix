{ pkgs, server }:

pkgs.runCommand "kikoeru-neo-static-check"
  { nativeBuildInputs = [ pkgs.binutils pkgs.python3 ]; }
  ''
    readelf -h ${server}/bin/kikoeru-neo
    if readelf -l ${server}/bin/kikoeru-neo | grep -q INTERP; then
      echo 'The static binary has an ELF interpreter.' >&2
      exit 1
    fi
    if readelf -d ${server}/bin/kikoeru-neo | grep -q NEEDED; then
      echo 'The static binary needs a shared library.' >&2
      exit 1
    fi
    python3 ${./static-check.py} ${server}/bin/kikoeru-neo
    touch $out
  ''
