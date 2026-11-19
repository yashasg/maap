# Maap operator runbook

**Pending operator work.** This directory does not provision infrastructure, publish images, or configure the edge. Start with an approved Linux host with Docker Engine and Compose v2, the released `linux/amd64` image, and a human-provided tunnel token, SDK URL and Access-protected dashboard URL. Do not start the connector until the dashboard is protected. The supplied route must leave `/v2` **and** `/v2/*` ungated: Next redirects the SDK's trailing-slash POST `/v2/` to `/v2`. `/mcp` bypasses Access but requires an Umami API key. No app or DB port is published; the connector reaches `http://umami:3000` on the Compose network.

## Run a release

**Visitor location through the tunnel.** On the Cloudflare zone that serves the SDK's public hostname, open **Rules → Settings → Managed Transforms** and enable [Add visitor location headers](https://developers.cloudflare.com/rules/transform/managed-transforms/reference/#add-visitor-location-headers). Cloudflare adds `CF-IPCountry`, `CF-Region-Code` and `CF-IPCity` (when known) to the origin request before `cloudflared` forwards it to Maap. No connector restart, app change or MaxMind database is required. **Network → IP Geolocation** alone supplies only the country.

Verify with a fresh SDK session sent to the public hostname. Open the session in Maap and check its country, region and city. These are estimates for the visitor's public IP (possibly a VPN exit), not the VM's location or device GPS. Requests made directly to the origin bypass Cloudflare. Changing the zone does not retroactively locate old visits.

Default-branch pushes by the repository owner automatically publish `ghcr.io/yashasg/umami-td:<git-sha>` and invoke `/opt/maap/deploy.sh <git-sha>` on `maap-vm` (`us-west1-b`) over Google Cloud IAP using Workload Identity Federation (OIDC). `deploy.sh` backs up a running database to `/var/backups/maap`, atomically updates `UMAMI_IMAGE_TAG` in `/opt/maap/.env`, pulls and restarts `db` and `umami`, verifies `/api/heartbeat`, and starts `cloudflared` once `TUNNEL_TOKEN` is configured.

For manual setup or inspection on the host:

1. Copy the contents of this directory from the approved release into a root-owned runtime directory **outside any checkout/build context**, e.g. `/opt/maap`. Keep this directory and the `maap` project name stable between releases so `pgdata` is reused. Run the following there as the stack operator. On **first install only**:

   ```sh
   umask 077
   test ! -e .env && install -m 0600 .env.example .env
   ```

2. Edit `.env` privately. Set `UMAMI_IMAGE_TAG` to the approved **full commit SHA** published for `ghcr.io/yashasg/umami-td`, not `latest` or a branch. Generate three independent 32-byte hex strings (`openssl rand -hex 32`): one each for `POSTGRES_PASSWORD`, `APP_SECRET`, and `TWO_FACTOR_ENCRYPTION_KEY`. Hex avoids URL escaping in `DATABASE_URL`; 2FA requires exactly 64 hex characters. Escrow secrets separately from DB dumps. Enter the provided `TUNNEL_TOKEN`; never paste tokens into a command, ticket or build log. Use `config --quiet`, not `config`, around real credentials.
3. Keep `TD_HOSTNAME=app.ios` (analytics label, **not** the public URL). Set `TD_ALLOWED_KEYS` to an explicit comma-separated list of exact payload keys to store (no wildcards). This **replaces** the defaults. Blank retains every non-`TelemetryDeck.*` key, including legacy metadata, plus the SDK version, build number, model name and system version. First-class dimensions and numeric `floatValue` are separate; filter sensitive data before sending. Leave `TD_TEST_WEBSITE_ID` blank until you have a separate test website UUID. Blank **drops** test signals; it does not send them to production.

   ```sh
   docker compose config --quiet
   docker compose pull
   docker compose up -d --wait --wait-timeout 240 db umami
   docker compose exec -T umami curl -fsS http://127.0.0.1:3000/api/heartbeat
   # Only after the supplied Access-protected route is ready:
   docker compose up -d cloudflared
   docker compose ps
   docker stats --no-stream
   ```

   Record the SHA and pulled image digest. The app runs DB migrations at startup. For subsequent releases, take and validate a backup first; change **only** the SHA in `.env`, `docker compose pull umami`, then `docker compose up -d --wait --wait-timeout 240 umami`. If unhealthy, check `docker compose logs --tail=80 umami db` privately and inspect memory/restart counts. Revert to the last healthy SHA only if the schema is compatible; otherwise plan recovery into a **fresh** volume from the pre-release backup. Never run `down -v` on production. Changing `POSTGRES_PASSWORD` in `.env` alone does not rotate a password in an existing volume.

## Bootstrap Umami and verify a release

Use the supplied protected URL, not a direct origin or a public port. These checks are **not yet performed**:

- [ ] `db` and `umami` healthy, connector running; no published ports. Confirm configured memory limits (`umami` 450 MiB, `db` 200 MiB, `cloudflared` 64 MiB) and observe available headroom under representative load. The Node heap limit (256 MiB) is not its total process memory.
- [ ] After the stack settles, idle host RAM usage is **below 85%** (use `100 × (1 − MemAvailable/MemTotal)` from `/proc/meminfo`). After an approved host reboot, confirm all three services return without manual intervention, the heartbeat succeeds, and existing website data is still visible.
- [ ] On first install sign in as the seeded `admin` / `umami` and immediately replace the default password with **at least 24 random characters**. In **Settings → Security**, enroll an authenticator and test a fresh login. Store the TOTP secret and recovery codes in the approved password manager; share with authorized operators **only through that manager**. To require 2FA for everyone, turn on **Admin → Security** and verify another user is prompted to enroll; configuring the encryption key alone does **not** enforce 2FA. Keep its value stable across deployments/restores.
- [ ] In **Websites → Add website**, create separate production and test websites for the app with domain `app.ios` (or your chosen `TD_HOSTNAME`). Copy their actual website UUIDs from website settings. Give the production UUID to the SDK owner as `appID`. Set `TD_TEST_WEBSITE_ID` to the **test** UUID and run `docker compose up -d --wait umami` to apply the change. Website IDs identify projects; they are not credentials.
- [ ] Using an **actual iOS debug build** configured for the supplied SDK URL and test mode, navigate three distinct screens and send one named custom event without identifying data. Verify the signals appear on the **test** website in **under 60 seconds** and not on production. Confirm three pageviews with distinct paths, the named event under **Events**, and `App <version>` under **Browser**. Record elapsed time; neither a synthetic request nor an HTTP 200 alone passes this check.
- [ ] For read-only MCP reporting, grant a dedicated user only the necessary website access. From that account's **Settings → API keys** create a key, store it privately, and use `Authorization: Bearer <key>` with the Streamable HTTP endpoint `<dashboard-origin>/mcp`. This path bypasses Access: the client supplies **only the Umami bearer API key**, not an Access token or a browser login token. Confirm a request without the key is rejected by Umami, then verify `get_website_stats` for the expected website and time range with the key. Revoke test keys; never ship keys in an app.
- [ ] Record release SHA/digest, test UTC time, website IDs, health and results (without secrets). Complete the backup and restore checks below before sign-off.

## Nightly database backup and restore rehearsal

Agree on **destination, owner, encryption, off-host copy, retention and cost** before enabling backups. A dump on the same VM/volume is not disaster recovery; storage, transfer and recovery hosts can incur charges. Use an approved private filesystem directory that supports locking and atomic rename, outside the DB volume and build context. Make it mode `0700`, owned by the account running the job. A custom-format `pg_dump` is compressed, **not encrypted**.

1. Set `BACKUP_DIR` to that absolute path. Run `bash /opt/maap/backup.sh` once with it exported. The job locks the directory, writes a private partial, checks the archive catalog and renames into place. Only on success does it prune completed Maap dumps older than seven days. It does not upload data or back up `.env`.
2. Install a **nightly** job in the stack operator's crontab; substitute the agreed directory. This example runs at 02:15 in the host's time zone. Configure actual failure notification and alert when the newest completed dump is more than 26 hours old; check free space and verify the off-host copy. Match the destination's lifecycle to the agreed retention.

   ```cron
   PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
   15 2 * * * BACKUP_DIR=/approved/private/maap /bin/bash /opt/maap/backup.sh
   ```

3. **Restore drill:** before go-live, after DB upgrades, and periodically, retrieve a trusted completed backup from the recovery destination onto a separate private host with adequate disk and the matching `postgres:16-alpine` image already present. Run:

   ```sh
   bash restore-drill.sh /approved/recovery/maap-YYYYMMDDTHHMMSSZ.dump
   ```

   This restores with error checking into a new isolated PostgreSQL container with no network peers, no published port, and its own disposable volume. Compare the website count, event presence and completed migrations to expectations for the backup date. A catalog listing alone is not a restore test. Record the backup timestamp, result and elapsed recovery time. The trap removes only the drill container and its volume on normal exit; confirm cleanup and securely expire the staged copy. Full service recovery also requires the recorded app release and separately escrowed secrets.
