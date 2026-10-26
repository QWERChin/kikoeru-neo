{ pkgs, server }:

let runtime = import ./docker-runtime.nix { inherit pkgs server; };
in pkgs.runCommand "kikoeru-neo-dockerfile-check"
  { nativeBuildInputs = [ pkgs.python3 ]; }
  ''
    python3 ${./dockerfile-check.py} ${../Dockerfile} \
      ${pkgs.writeText "image-config.json" (builtins.toJSON runtime.config)}
    touch $out
  ''
