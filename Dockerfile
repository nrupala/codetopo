# codetopo — self-hosted image.
#
# Multi-stage: build the four binaries in the Rust image, ship them
# on a slim Debian runtime. The binaries are self-contained
# (rusqlite is bundled; ldd shows only libc/libm/libgcc), so the
# runtime needs nothing else.
#
# The server is configured entirely by environment (see
# crates/codetopo-server/src/config.rs):
#   CODETOPO_BIND         default 127.0.0.1:8080 — the image sets
#                         0.0.0.0:8080 so the port can be published.
#   CODETOPO_DATA_DIR     where index databases live (/data).
#   CODETOPO_KEYS_FILE    newline `key_id:secret` lines; REQUIRED —
#                         the server refuses to start without it.
#   CODETOPO_METERING_LOG append-only request metering log.
#
# Quick start: see docker-compose.yml.

FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked \
    && strip target/release/codetopo target/release/codetopo-server \
             target/release/codetopo-mcp target/release/codetopo-acp

FROM debian:bookworm-slim
RUN useradd --system --home-dir /data --shell /usr/sbin/nologin codetopo \
    && mkdir -p /data && chown codetopo:codetopo /data
COPY --from=build /src/target/release/codetopo /usr/local/bin/codetopo
COPY --from=build /src/target/release/codetopo-server /usr/local/bin/codetopo-server
COPY --from=build /src/target/release/codetopo-mcp /usr/local/bin/codetopo-mcp
COPY --from=build /src/target/release/codetopo-acp /usr/local/bin/codetopo-acp
ENV CODETOPO_BIND=0.0.0.0:8080 \
    CODETOPO_DATA_DIR=/data \
    CODETOPO_KEYS_FILE=/data/api_keys \
    CODETOPO_METERING_LOG=/data/metering.log
VOLUME ["/data"]
EXPOSE 8080
USER codetopo
WORKDIR /data
# Default: the HTTP API. Index into the same volume from a one-off
# run:  docker compose run --rm server codetopo index /repos/x --db /data/x.db
# (mount the code tree at /repos first — see docker-compose.yml).
CMD ["codetopo-server"]
