# Copyright (C) 2026 2351485 ALBERTA INC.
# SPDX-License-Identifier: AGPL-3.0-or-later
# Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.
#
# codetopo — Hugging Face Docker Space image.
#
# BUILD CONTEXT: this Dockerfile is written for the ASSEMBLED Space layout,
# not for the GitHub tree. The hf-sync workflow (.github/workflows/hf-sync.yml)
# assembles the Space repository as the whole codetopo repo tree with the
# contents of space/ overlaid at the root, so at build time this file sits at
# the repository root next to Cargo.toml and crates/. Building it from the
# GitHub checkout directly will not work — assemble first (see the workflow).
#
# The demo it produces is read-only by construction: at container start the
# entrypoint indexes the bundled copy of codetopo's own source with a
# generated, container-local API key, then serves only the API's GET query
# endpoints through a small front door. The key never leaves the container.

# ---- Stage 1: build the API server and prepare the pristine demo corpus ----
FROM rust:1-bookworm AS builder
WORKDIR /build
COPY . .
# Only the server binary is needed at runtime; the CLI/MCP/ACP binaries are
# not part of the Space.
RUN cargo build --release --locked --package codetopo-server
# Pristine source copy for the demo corpus: the same tree, minus build
# output and VCS metadata. (The indexer also skips target/ and .git on its
# own; excluding them here keeps the runtime image lean.)
RUN mkdir -p /srv/codetopo-src \
 && tar --exclude='./target' --exclude='./.git' -cf - . | tar -xf - -C /srv/codetopo-src

# ---- Stage 2: runtime ----
FROM debian:bookworm-slim
# Spaces run the container as UID 1000; create that user up front so file
# ownership is right without recursive chowns.
RUN useradd -m -u 1000 user \
 && apt-get update \
 && apt-get install -y --no-install-recommends python3 curl ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && mkdir -p /data \
 && chown user:user /data
COPY --from=builder /build/target/release/codetopo-server /usr/local/bin/codetopo-server
COPY --from=builder /srv/codetopo-src /srv/codetopo-src
COPY --chown=user frontdoor.py index.html entrypoint.sh /home/user/app/
ENV HOME=/home/user
USER user
WORKDIR /home/user/app
EXPOSE 8787
CMD ["sh", "entrypoint.sh"]
