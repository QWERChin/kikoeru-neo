FROM nixos/nix:2.31.2@sha256:29fc5fe207f159ceb0143c25c19c774062fee02ce5eda118f3067547b3054894 AS nix
FROM debian:bookworm-slim@sha256:7ff6ba31527b9e336162fa061746c366c4b2032f4d2cff2078bea43f78efe3f8

# IDE servers need the distribution's glibc loader and conventional paths.
RUN apt-get update && apt-get install -y --no-install-recommends \
      bash ca-certificates curl git libstdc++6 openssh-client procps python3 sudo xz-utils \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 1000 dev \
    && useradd --uid 1000 --gid 1000 --create-home --shell /bin/bash dev \
    && printf 'dev ALL=(ALL) NOPASSWD:ALL\n' > /etc/sudoers.d/dev
# Set ownership before building the closure, so no later layer copies it.
COPY --from=nix --chown=1000:1000 /nix /nix
ENV PATH=/nix/var/nix/profiles/default/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    NIX_SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt
RUN mkdir -p /etc/nix \
    && printf 'experimental-features = nix-command flakes\nsandbox = false\nbuild-users-group =\n' > /etc/nix/nix.conf

RUN install -d -o dev -g dev /opt/flake
USER 1000:1000
ENV USER=dev HOME=/home/dev
WORKDIR /opt/flake
# Only the dev shell's evaluation inputs invalidate its expensive layer.
COPY --chown=1000:1000 flake.nix flake.lock ./
COPY --chown=1000:1000 nix/dev-shell.nix nix/fallback-fonts.nix nix/fallback-fonts.py ./nix/
RUN nix develop --profile /nix/var/nix/profiles/dev-shell -c true \
    && nix print-dev-env --json > /opt/flake/dev-env.json

USER root
COPY .devcontainer/export-env.py .devcontainer/dev-command .devcontainer/dev-sh /opt/setup/
RUN python3 /opt/setup/export-env.py /opt/flake/dev-env.json /opt/dev-env.sh /opt/dev/bin \
    && install -m 755 /opt/setup/dev-command /usr/local/bin/dev-command \
    && install -m 755 /opt/setup/dev-sh /usr/local/bin/dev-sh \
    && ln -sf /usr/local/bin/dev-sh /bin/sh \
    && printf '\n. /opt/dev-env.sh\n' >> /etc/bash.bashrc \
    && printf '. /opt/dev-env.sh\n' > /etc/profile.d/dev-env.sh \
    && mkdir -p /workspace \
    && chown dev:dev /workspace \
    && rm -rf /opt/setup
ENV PATH=/opt/dev/bin:/nix/var/nix/profiles/default/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    BASH_ENV=/opt/dev-env.sh \
    ENV=/opt/dev-env.sh
USER 1000:1000
ENV USER=dev HOME=/home/dev
WORKDIR /workspace
ENTRYPOINT ["/usr/local/bin/dev-command"]
CMD ["/bin/bash", "-l"]
