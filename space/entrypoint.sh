#!/bin/sh
# Copyright (C) 2026 Nrupal Akolkar
# SPDX-License-Identifier: AGPL-3.0-or-later
# Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.
#
# Container entrypoint for the codetopo Hugging Face Space.
#
#   1. Generate a demo API key that exists only inside this container.
#   2. Start codetopo-server on loopback (127.0.0.1:8080) — never exposed.
#   3. Wait until the authenticated API answers.
#   4. Index the bundled copy of codetopo's own source (self-analysis).
#   5. Exec the read-only front door on the public port (8787); it becomes
#      PID 1, so the container's lifecycle follows it.
#
# The demo key is generated fresh on every container start, is written only
# to the keys file inside the container's data directory, and is never
# printed, served, or baked into the image.
set -eu

APP_DIR=/home/user/app
SRC_DIR=${CODETOPO_SRC_DIR:-/srv/codetopo-src}
DATA_DIR=${CODETOPO_DATA_DIR:-/data}
KEYS_FILE="$DATA_DIR/api_keys"
API=http://127.0.0.1:8080

export CODETOPO_BIND=127.0.0.1:8080
export CODETOPO_DATA_DIR="$DATA_DIR"
export CODETOPO_KEYS_FILE="$KEYS_FILE"
export CODETOPO_METERING_LOG="$DATA_DIR/metering.log"

mkdir -p "$DATA_DIR"

# 1. Demo key (key id "demo"; the secret is random per container start).
SECRET=$(python3 -c 'import secrets; print(secrets.token_hex(24))')
printf 'demo:%s\n' "$SECRET" > "$KEYS_FILE"
chmod 600 "$KEYS_FILE"

# 2. The API server, loopback only. It refuses to start without a keys
#    file, and every route — including /v1/schema — requires the key.
codetopo-server &

# 3. Readiness probe (authenticated; the server has no open health route).
i=0
until curl -fs -H "Authorization: Bearer demo:$SECRET" "$API/v1/schema" >/dev/null 2>&1; do
  i=$((i + 1))
  if [ "$i" -gt 60 ]; then
    echo "entrypoint: codetopo-server did not become ready in 60s" >&2
    exit 1
  fi
  sleep 1
done

# 4. Index codetopo's own source, bundled in the image. This is the only
#    index operation the demo ever performs, and it happens before the
#    public port opens.
INDEX_JSON=$(curl -fs -X POST "$API/v1/index" \
  -H "Authorization: Bearer demo:$SECRET" \
  -H 'Content-Type: application/json' \
  -d "{\"repo_path\": \"$SRC_DIR\", \"package\": \"codetopo\"}")
INDEX_ID=$(printf '%s' "$INDEX_JSON" | python3 -c 'import json, sys; print(json.load(sys.stdin)["index_id"])')
echo "entrypoint: indexed $SRC_DIR as index $INDEX_ID"

# 5. Front door (read-only proxy + landing page) on the public port.
export CODETOPO_API_BASE="$API"
export CODETOPO_DEMO_KEY="demo:$SECRET"
export CODETOPO_INDEX_ID="$INDEX_ID"
export FRONTDOOR_PORT=8787
export FRONTDOOR_DIR="$APP_DIR"
exec python3 "$APP_DIR/frontdoor.py"
