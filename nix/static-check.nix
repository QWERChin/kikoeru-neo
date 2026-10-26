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
    # The prefixed jemalloc reads this variable; without it the build fell
    # back to musl's own malloc.
    if ! grep -q _RJEM_MALLOC_CONF ${server}/bin/kikoeru-neo; then
      echo 'The static binary does not use jemalloc.' >&2
      exit 1
    fi
    python3 ${./static-check.py} ${server}/bin/kikoeru-neo
    touch $out
  ''
