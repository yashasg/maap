#!/bin/sh
# Container startup: run the one-shot DB check/migration and tracker update, then
# exec the long-running server so it replaces this shell as PID 1.
#
# This avoids `npm-run-all` keeping idle `npm`/`npm-run-all` parent processes
# resident for the lifetime of the container (each a full Node VM holding tens
# of MiB of anonymous memory). By default only maap-server stays running.
#
# PATH is extended so check-db.js's `prisma migrate deploy` resolves the prisma
# CLI from node_modules/.bin the way `npm run` would have injected it.
set -e
export PATH="$PWD/node_modules/.bin:$PATH"

node scripts/check-db.js
node scripts/update-tracker.js
node scripts/fetch-font.mjs

# Next's standalone server binds to $HOSTNAME. Some runtimes (e.g. Podman pods)
# overwrite it with the container hostname, which binds only to the pod IP and
# breaks localhost healthchecks, so force all interfaces here.
case "${MAAP_SERVER:-rust}" in
  node) export HOSTNAME=0.0.0.0; exec node server.js ;;
  rust) ;;
  *) echo "MAAP_SERVER must be node or rust" >&2; exit 2 ;;
esac

if [ "${MAAP_NODE_FALLBACK:-0}" = "1" ]; then
  HOSTNAME=127.0.0.1 PORT="${MAAP_NODE_PORT:-3001}" node server.js &
fi

export HOSTNAME=0.0.0.0
exec ./maap-server
