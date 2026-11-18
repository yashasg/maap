#!/bin/bash
# Use on an isolated recovery host with the DB image already pulled.
set -euo pipefail
[[ $# == 1 && -r $1 && -s $1 ]] || { echo "Usage: $0 trusted-backup.dump" >&2; exit 2; }

# No live Compose project, ports, network peers, host mounts or production credentials.
POSTGRES_PASSWORD=$(openssl rand -hex 32)
export POSTGRES_PASSWORD
drill=$(docker run --pull=never -d --network none \
  -e POSTGRES_PASSWORD -e POSTGRES_USER=umami -e POSTGRES_DB=maap_restore \
  -v /var/lib/postgresql/data postgres:16-alpine -c listen_addresses=127.0.0.1)
trap 'docker rm -fv "$drill" >/dev/null' EXIT
trap 'exit 1' HUP INT TERM
unset POSTGRES_PASSWORD

# Test TCP; the temporary server during initdb accepts Unix sockets only.
for ((i = 0; i < 60; i++)); do
  if docker exec "$drill" pg_isready -qh 127.0.0.1 -U umami -d maap_restore; then break; fi
  sleep 2
done
docker exec "$drill" pg_isready -qh 127.0.0.1 -U umami -d maap_restore
docker exec -i "$drill" pg_restore --exit-on-error --single-transaction --no-owner --no-acl \
  -U umami -d maap_restore < "$1"
docker exec "$drill" psql -X -v ON_ERROR_STOP=1 -U umami -d maap_restore -c \
  'SELECT (SELECT count(*) FROM website) AS websites,
          EXISTS (SELECT 1 FROM website_event) AS has_events,
          (SELECT count(*) FROM _prisma_migrations WHERE finished_at IS NOT NULL) AS migrations;'
