#!/bin/sh
set -eu
cd -- "$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
if [ ! -f connector.json ]; then
    printf '%s\n' 'Download the node installation config from the center as connector.json first.' >&2
    exit 1
fi
if [ -z "${SSL_CERT_FILE:-}" ] && [ -f testflying-prod-ca.crt ]; then
    SSL_CERT_FILE="$(pwd)/testflying-prod-ca.crt"
    export SSL_CERT_FILE
fi
exec ./store-connector connector.json
