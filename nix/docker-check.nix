# Checks the Docker image's configuration and file system without a Docker
# daemon. The measured layer sizes end up in $out/sizes.txt.
{ pkgs, image, server }:

pkgs.runCommand "kikoeru-neo-docker-image-check"
  {
    nativeBuildInputs = [ pkgs.python3 ];
  }
  ''
    python3 -I ${./docker-check.py} ${image} ${server} \
      ${image.passthru.entrypoint} ${image.passthru.healthcheck}/bin/kikoeru-neo-healthcheck $out
  ''
