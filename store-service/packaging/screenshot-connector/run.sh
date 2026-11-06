#!/bin/sh
set -eu
umask 077

root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
[ "$(uname -s)" = Darwin ] || { echo 'Screenshot Connector requires macOS.' >&2; exit 1; }
python=${SCREENSHOT_CONNECTOR_PYTHON:-$(command -v python3)}
tools=${SCREENSHOT_CONNECTOR_LIFECYCLE_BIN:-"$HOME/Library/Application Support/CodexArtifactLifecycle/bin"}
"$python" -B -c 'import sys; raise SystemExit(sys.version_info < (3, 11))' ||
    { echo 'Screenshot Connector requires Python 3.11 or newer.' >&2; exit 1; }
"$tools/artifact-audit" --self-check >/dev/null ||
    { echo 'Artifact lifecycle integration must be installed and active.' >&2; exit 1; }
if [ "${1-}" = --service ] && [ "$#" -eq 1 ]; then
    SCREENSHOT_CONNECTOR_PYTHON=$python
    SCREENSHOT_CONNECTOR_LIFECYCLE_BIN=$tools
    custom_ca=$(/usr/bin/plutil -extract caFile raw -expect string -o - -- "$root/connector.json" 2>/dev/null || true)
    SSL_CERT_FILE=${custom_ca:-${SSL_CERT_FILE:-"$root/testflying-prod-ca.crt"}}
    export SCREENSHOT_CONNECTOR_PYTHON SCREENSHOT_CONNECTOR_LIFECYCLE_BIN SSL_CERT_FILE
    exec "$root/screenshot-connector" "$root/connector.json"
fi
[ "$#" -eq 0 ] || { echo 'Usage: run.sh [--service]' >&2; exit 1; }
exec "$tools/artifact-run" --adapter generic --project "$root" -- "$python" -B \
    "$root/scripts/screenshot_connector.py" "$root/connector.json"
