#!/bin/sh
set -eu
umask 077

root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
[ "$(uname -s)" = Darwin ] || { echo 'Screenshot Connector requires macOS.' >&2; exit 1; }
custom_ca=$(/usr/bin/plutil -extract caFile raw -expect string -o - -- "$root/connector.json" 2>/dev/null || true)
SSL_CERT_FILE=${custom_ca:-${SSL_CERT_FILE:-"$root/testflying-prod-ca.crt"}}
export SSL_CERT_FILE
if [ "${1-}" = --service ] && [ "$#" -eq 1 ]; then
    exec "$root/screenshot-connector" "$root/connector.json"
fi
[ "$#" -eq 0 ] || { echo 'Usage: run.sh [--service]' >&2; exit 1; }
exec "$root/screenshot-connector" --once "$root/connector.json"
