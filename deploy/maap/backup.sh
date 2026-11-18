#!/bin/bash
# Run as the stack operator. Write only to an existing, dedicated private directory.
set -euo pipefail
umask 077
cd -- "$(dirname -- "$0")"
: "${BACKUP_DIR:?Set BACKUP_DIR to your approved absolute backup directory}"
[[ $BACKUP_DIR == /* && -d $BACKUP_DIR && -O $BACKUP_DIR && $(stat -c %a -- "$BACKUP_DIR") == 700 ]] ||
  { echo "BACKUP_DIR must exist, be owned by this user and have mode 0700" >&2; exit 1; }

exec 9<"$BACKUP_DIR"
flock -n 9 || { echo "Backup already in progress" >&2; exit 1; }
partial="$BACKUP_DIR/.maap.dump.partial"
final="$BACKUP_DIR/maap-$(date -u +%Y%m%dT%H%M%SZ).dump"
trap 'rm -f -- "$partial"' EXIT
trap 'exit 1' HUP INT TERM

docker compose exec -T db pg_dump -Fc -U umami -d umami > "$partial"
docker compose exec -T db pg_restore --list < "$partial" > /dev/null
mv -T -- "$partial" "$final"
# Expire only this job's completed files, and only after publishing a new one.
find "$BACKUP_DIR" -maxdepth 1 -type f -name 'maap-????????T??????Z.dump' -mmin +10080 -delete
printf 'Completed %s\n' "$final"
