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
exec "$tools/artifact-run" --adapter generic --project "$root" -- "$python" -B \
    "$root/scripts/screenshot_connector.py" "$root/connector.json"
