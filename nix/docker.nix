# Streams layers without keeping a complete image archive in the store.
{ pkgs, server, created }:

let runtime = import ./docker-runtime.nix { inherit pkgs server; };
in pkgs.dockerTools.streamLayeredImage {
  name = "kikoeru-neo";
  tag = server.version;
  inherit created;
  maxLayers = 120;
  contents = [ runtime.root ];
  inherit (runtime) config fakeRootCommands;
  passthru = {
    entrypoint = "${runtime.entrypoint}/bin/kikoeru-neo-entrypoint";
    inherit (runtime) healthcheck;
  };
}
