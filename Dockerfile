# The index pins both native amd64 and arm64 builders.
FROM nixos/nix:2.31.2@sha256:29fc5fe207f159ceb0143c25c19c774062fee02ce5eda118f3067547b3054894 AS builder
WORKDIR /src
COPY . .
RUN nix --extra-experimental-features 'nix-command flakes' build \
      --option sandbox false --print-build-logs -o /tmp/rootfs .#docker-rootfs \
    && mkdir /runtime \
    && tar -xpf /tmp/rootfs/rootfs.tar -C /runtime

FROM scratch
COPY --from=builder /runtime/ /
ENTRYPOINT ["/bin/tini", "--", "/bin/kikoeru-neo-entrypoint"]
CMD ["serve", "--config", "/config/config.toml"]
USER 1000:1000
ENV KIKOERU_DATA_DIR=/data \
    KIKOERU_LISTEN=0.0.0.0:8888 \
    SQLITE_TMPDIR=/data/cache/tmp \
    MALLOC_ARENA_MAX=2
EXPOSE 8888/tcp
VOLUME ["/data", "/backup"]
HEALTHCHECK --interval=30s --timeout=10s --start-period=120s --start-interval=2s --retries=3 CMD ["/bin/kikoeru-neo-healthcheck"]
LABEL org.opencontainers.image.version="0.1.0" \
      org.opencontainers.image.source="https://github.com/QWERChin/kikoeru-neo" \
      org.opencontainers.image.licenses="GPL-3.0-or-later"
