# An archive preserves numeric owners that a Nix store directory cannot.
{ pkgs, server }:

let
  runtime = import ./docker-runtime.nix { inherit pkgs server; };
  closure = pkgs.closureInfo { rootPaths = [ runtime.root ]; };
in pkgs.runCommand "kikoeru-neo-docker-rootfs"
  { nativeBuildInputs = [ pkgs.fakeroot ]; }
  ''
    mkdir -p root/nix/store $out
    while read -r path; do
      cp -a "$path" root/nix/store/
    done < ${closure}/store-paths
    cp -a ${runtime.root}/. root/
    chmod u+w root
    fakeroot bash -eu <<'EOF'
    cd root
    ${runtime.fakeRootCommands}
    tar --sort=name --mtime=@0 --numeric-owner -cf $out/rootfs.tar .
    EOF
  ''
