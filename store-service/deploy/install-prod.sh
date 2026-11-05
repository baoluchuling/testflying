#!/usr/bin/env bash
set -euo pipefail

: "${STORE_API_PUBLIC_URL:?Provide the real Store API HTTPS public URL privately}"

# First installation only. Updates require reviewing the existing production state.
RELEASE_DIR="${1:?Provide the canonical uploaded release directory}"
case "$RELEASE_DIR" in
  /opt/testflying-store-api/versions/[0-9a-f]*) ;;
  *) echo 'Invalid release directory' >&2; exit 64 ;;
esac
[[ "$(realpath "$RELEASE_DIR")" == "$RELEASE_DIR" ]]
[[ ! -e /etc/systemd/system/testflying-store-api.service ]]
[[ ! -e /etc/testflying-store-api/service.env ]]
[[ ! -e /opt/testflying-store-api/current ]]
[[ ! -L /opt/testflying-store-api/current ]]
[[ "$(uname -m)" == x86_64 ]]
[[ "$(df --output=avail -B1 /opt | tail -1)" -gt 21474836480 ]]
[[ -z "$(ss -lnt '( sport = :8200 )' | tail -n +2)" ]]
cd "$RELEASE_DIR"
sha256sum --check SHA256SUMS

PROXY_CONFIG=/etc/bugsmith/zentao-tls/nginx.conf
BACKUP=/opt/testflying-store-api/nginx.before-store-api.conf
[[ -f "$PROXY_CONFIG" && ! -L "$PROXY_CONFIG" && ! -e "$BACKUP" ]]
[[ "$(grep -c 'location / {' "$PROXY_CONFIG")" == 1 ]]
! grep -q '/store-api/' "$PROXY_CONFIG"
cp -p "$PROXY_CONFIG" "$BACKUP"
chmod 600 "$BACKUP"

if ! id testflying-store >/dev/null 2>&1; then
  useradd --system --user-group --no-create-home --shell /sbin/nologin testflying-store
fi
install -d -m 700 /etc/testflying-store-api
install -d -m 700 -o testflying-store -g testflying-store /var/lib/testflying-store-api
# Environment file is a formal production secret, never printed or copied to logs.
(umask 077
  {
    echo 'STORE_API_LISTEN=127.0.0.1:8200'
    echo 'STORE_API_DATABASE=/var/lib/testflying-store-api/registry.sqlite3'
    printf 'STORE_API_PUBLIC_URL=%s\n' "$STORE_API_PUBLIC_URL"
    echo 'STORE_API_CONNECTOR_RELEASE_ROOT=/opt/testflying-store-api/current/releases'
    echo 'STORE_API_REQUEST_TIMEOUT_SECONDS=60'
    echo 'TOKIO_WORKER_THREADS=2'
    printf 'STORE_API_ADMIN_TOKEN=%s\n' "$(openssl rand -hex 32)"
    printf 'STORE_API_TOKEN=%s\n' "$(openssl rand -hex 32)"
    printf 'STORE_API_NODE_SIGNING_KEY=%s\n' "$(openssl rand -hex 48)"
  } > /etc/testflying-store-api/service.env
)
ln -s "$RELEASE_DIR" /opt/testflying-store-api/current
install -m 644 deploy/testflying-store-api.service /etc/systemd/system/testflying-store-api.service
systemctl daemon-reload
systemctl enable --now testflying-store-api
for attempt in {1..20}; do
  if curl --fail --silent http://127.0.0.1:8200/health >/dev/null; then break; fi
  sleep 1
done
curl --fail --silent http://127.0.0.1:8200/health

# Write the existing bind-mounted file in place so the proxy container sees it.
python3 - "$PROXY_CONFIG" <<'PY'
import sys
path = sys.argv[1]
with open(path) as file:
    previous = file.read()
needle = '        location / {'
if previous.count(needle) != 1:
    raise SystemExit('Unexpected proxy configuration')
location = '''        location = /store-api {
            return 308 /store-api/;
        }

        location /store-api/ {
            proxy_pass http://127.0.0.1:8200/;
            proxy_http_version 1.1;
            proxy_set_header Upgrade $http_upgrade;
            proxy_set_header Connection "upgrade";
            proxy_set_header Host $host;
            proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
            proxy_set_header X-Forwarded-Proto https;
            proxy_read_timeout 1860s;
            proxy_send_timeout 90s;
            proxy_request_buffering off;
            client_body_buffer_size 64k;
            client_max_body_size 64m;
        }

'''
with open(path, 'w') as file:
    file.write(previous.replace(needle, location + needle))
PY
if ! docker exec zentao-tls-proxy nginx -t; then
  cat "$BACKUP" > "$PROXY_CONFIG"
  echo 'Proxy validation failed; original configuration restored' >&2
  exit 1
fi
docker exec zentao-tls-proxy nginx -s reload
curl --fail --silent --cacert /etc/bugsmith/zentao-tls/certs/ca.crt \
  https://store-api.example.com/store-api/health
systemctl is-active testflying-store-api
