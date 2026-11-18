#!/bin/bash
# ponytail: single-host Compose deploy script called over IAP SSH by GitHub Actions
set -euo pipefail
umask 077
cd -- "$(dirname -- "$0")"

tag="${1:-}"
[[ $# == 1 && $tag =~ ^[0-9a-f]{40}$ ]] || { echo "Usage: $0 <40-hex-commit-sha>" >&2; exit 2; }

if [[ ! -f .env ]]; then
  cat > .env <<EOF
UMAMI_IMAGE=ghcr.io/yashasg/umami-td
UMAMI_IMAGE_TAG=${tag}
POSTGRES_PASSWORD=$(openssl rand -hex 32)
APP_SECRET=$(openssl rand -hex 32)
TWO_FACTOR_ENCRYPTION_KEY=$(openssl rand -hex 32)
TUNNEL_TOKEN=pending-cloudflare-tunnel-token
TD_HOSTNAME=app.ios
TD_TEST_WEBSITE_ID=
TD_ALLOWED_KEYS=
EOF
else
  awk -v tag="$tag" '
    BEGIN { updated = 0 }
    /^UMAMI_IMAGE_TAG=/ { print "UMAMI_IMAGE_TAG=" tag; updated = 1; next }
    { print }
    END { if (!updated) print "UMAMI_IMAGE_TAG=" tag }
  ' .env > .env.tmp
  chmod 0600 .env.tmp
  mv -f .env.tmp .env
fi
chmod 0600 .env

if [[ -n "${GHCR_TOKEN:-}" ]]; then
  printf '%s' "$GHCR_TOKEN" | docker login ghcr.io -u "${GHCR_USER:-yashasg}" --password-stdin
  trap 'docker logout ghcr.io >/dev/null 2>&1 || true' EXIT
  unset GHCR_TOKEN
fi

if [[ -n "$(docker compose ps -q db 2>/dev/null || true)" ]] && docker compose exec -T db pg_isready -qh 127.0.0.1 -U umami -d umami 2>/dev/null; then
  BACKUP_DIR="${BACKUP_DIR:-/var/backups/maap}"
  if [[ -d $BACKUP_DIR ]]; then
    BACKUP_DIR="$BACKUP_DIR" bash ./backup.sh
  fi
fi

docker compose config --quiet
docker compose pull db umami
docker compose up -d --wait --wait-timeout 240 db umami
docker compose exec -T umami curl -fsS --max-time 4 http://127.0.0.1:3000/api/heartbeat >/dev/null

tunnel_token="$(awk -F= '/^TUNNEL_TOKEN=/{sub(/^[^=]*=/, ""); print; exit}' .env)"
if [[ -n $tunnel_token && $tunnel_token != "pending-cloudflare-tunnel-token" ]]; then
  docker compose pull cloudflared
  docker compose up -d cloudflared
fi

docker image prune -f >/dev/null
printf 'Deployed %s\n' "$tag"
