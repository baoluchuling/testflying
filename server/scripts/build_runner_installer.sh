#!/usr/bin/env bash
set -euo pipefail
umask 077

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
RUNNER_DIR="${REPO_ROOT}/build-runner"
PACKAGE_AGENT_DIR="${REPO_ROOT}/package-agent"
PACKAGING_DIR="${RUNNER_DIR}/packaging"
SYSTEM_INSTALL_ROOT="/Library/Application Support/TestFlying/build-runner"

if [[ $# -ne 3 ]]; then
  echo "Usage: $0 CONFIG_SOURCE OUTPUT_DIR VERSION" >&2
  exit 2
fi

CONFIG_SOURCE="$1"
OUTPUT_DIR="$2"
VERSION="$3"

if [[ ! "${VERSION}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "VERSION must use major.minor.patch format" >&2
  exit 2
fi
if [[ ! -f "${CONFIG_SOURCE}" || ! -r "${CONFIG_SOURCE}" ]]; then
  echo "CONFIG_SOURCE must be a readable regular file" >&2
  exit 2
fi
if [[ -e "${OUTPUT_DIR}" ]]; then
  echo "OUTPUT_DIR already exists: ${OUTPUT_DIR}" >&2
  exit 2
fi

for command in go python3.11 ditto shasum pkgbuild uname; do
  if ! command -v "${command}" >/dev/null 2>&1; then
    echo "Missing required build tool: ${command}" >&2
    exit 2
  fi
done

TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/testflying-runner-package.XXXXXX")"
trap 'rm -rf "${TMP_ROOT}"' EXIT
VALIDATED_CONFIG="${TMP_ROOT}/config.json"

python3.11 - "${CONFIG_SOURCE}" "${VALIDATED_CONFIG}" "${VERSION}" "${SYSTEM_INSTALL_ROOT}" <<'PY'
import json
import os
import sys
from pathlib import Path
from urllib.parse import urlsplit

source = Path(sys.argv[1])
destination = Path(sys.argv[2])
version = sys.argv[3]
install_root = sys.argv[4]

try:
    payload = json.loads(source.read_text(encoding="utf-8"))
except (OSError, UnicodeError, json.JSONDecodeError) as error:
    raise SystemExit(f"Invalid runner config JSON: {error}") from error
if not isinstance(payload, dict):
    raise SystemExit("Runner config must be a JSON object")

for field in ("runnerId", "name", "token", "serverUrl", "rootDir"):
    value = payload.get(field)
    if not isinstance(value, str) or not value.strip():
        raise SystemExit(f"Runner config field {field} must be a non-empty string")
    payload[field] = value.strip()

server_url = urlsplit(payload["serverUrl"])
if server_url.scheme not in {"http", "https"} or not server_url.netloc:
    raise SystemExit("Runner config field serverUrl must be an absolute HTTP(S) URL")
if not os.path.isabs(payload["rootDir"]):
    raise SystemExit("Runner config field rootDir must be absolute")

for field in ("labels",):
    values = payload.get(field)
    if not isinstance(values, list):
        raise SystemExit(f"Runner config field {field} must be a non-empty string list")
    normalized = [str(value).strip() for value in values if str(value).strip()]
    if not normalized:
        raise SystemExit(f"Runner config field {field} must be a non-empty string list")
    payload[field] = normalized

llm_adapters = payload.get("llmAdapters", [])
if not isinstance(llm_adapters, list):
    raise SystemExit("Runner config field llmAdapters must be a string list")
payload["llmAdapters"] = [str(value).strip() for value in llm_adapters if str(value).strip()]

capacity = payload.get("capacity")
if isinstance(capacity, bool) or capacity != 1:
    raise SystemExit("Runner config capacity must be 1")

payload["packageAgentBin"] = f"{install_root}/package-agent"
payload["version"] = version
payload["packageAgentVersion"] = version
destination.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
PY

RAW_ARCH="$(uname -m)"
case "${RAW_ARCH}" in
  arm64) ARCH="arm64" ;;
  x86_64|amd64) ARCH="amd64" ;;
  *)
    echo "Unsupported macOS architecture: ${RAW_ARCH}" >&2
    exit 2
    ;;
esac

FINAL_DIR="${TMP_ROOT}/final"
BIN_DIR="${TMP_ROOT}/bin"
FALLBACK_DIR="${FINAL_DIR}/build-runner-macos"
RELEASE_DIR="${FINAL_DIR}/darwin/${ARCH}"
PAYLOAD_ROOT="${TMP_ROOT}/payload"
PAYLOAD_INSTALL_DIR="${PAYLOAD_ROOT}${SYSTEM_INSTALL_ROOT}"
PACKAGE_SCRIPTS="${TMP_ROOT}/package-scripts"
BUNDLE_SOURCE="${TMP_ROOT}/update-bundle"
mkdir -p \
  "${BIN_DIR}" \
  "${FALLBACK_DIR}" \
  "${RELEASE_DIR}" \
  "${PAYLOAD_INSTALL_DIR}" \
  "${PACKAGE_SCRIPTS}" \
  "${BUNDLE_SOURCE}"

(
  cd "${RUNNER_DIR}"
  go build -trimpath -ldflags "-s -w" -o "${BIN_DIR}/testflying-build-runner" \
    ./cmd/testflying-build-runner
)

python3.11 -m PyInstaller \
  --clean \
  --onefile \
  --name package-agent \
  --distpath "${BIN_DIR}" \
  --workpath "${TMP_ROOT}/pyinstaller-work" \
  --specpath "${TMP_ROOT}" \
  --paths "${PACKAGE_AGENT_DIR}/src" \
  "${PACKAGE_AGENT_DIR}/src/package_agent/__main__.py"

chmod 755 "${BIN_DIR}/testflying-build-runner" "${BIN_DIR}/package-agent"
cp "${BIN_DIR}/testflying-build-runner" "${BIN_DIR}/package-agent" "${FALLBACK_DIR}/"
cp "${VALIDATED_CONFIG}" "${FALLBACK_DIR}/config.json"
cp "${PACKAGING_DIR}/install.command" "${FALLBACK_DIR}/install.command"
chmod 600 "${FALLBACK_DIR}/config.json"
chmod 755 "${FALLBACK_DIR}/install.command" \
  "${FALLBACK_DIR}/testflying-build-runner" \
  "${FALLBACK_DIR}/package-agent"

cp "${BIN_DIR}/testflying-build-runner" "${BIN_DIR}/package-agent" "${BUNDLE_SOURCE}/"
BUNDLE_NAME="testflying-runner-${VERSION}-darwin-${ARCH}.zip"
BUNDLE_PATH="${RELEASE_DIR}/${BUNDLE_NAME}"
ditto -c -k --norsrc --noextattr "${BUNDLE_SOURCE}" "${BUNDLE_PATH}"
CHECKSUM="$(shasum -a 256 "${BUNDLE_PATH}" | awk '{print tolower($1)}')"
printf '%s  %s\n' "${CHECKSUM}" "${BUNDLE_NAME}" > "${BUNDLE_PATH}.sha256"

python3.11 - "${RELEASE_DIR}/release.json" "${VERSION}" "${ARCH}" "${BUNDLE_NAME}" "${CHECKSUM}" <<'PY'
import json
import os
import sys
from pathlib import Path

destination = Path(sys.argv[1])
version, arch, bundle, checksum = sys.argv[2:]
payload = {
    "version": version,
    "runnerVersion": version,
    "packageAgentVersion": version,
    "platform": "darwin",
    "arch": arch,
    "bundleFile": bundle,
    "sha256": checksum.lower(),
}
temporary = destination.with_suffix(".json.tmp")
temporary.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
os.replace(temporary, destination)
PY

cp "${BIN_DIR}/testflying-build-runner" "${BIN_DIR}/package-agent" "${PAYLOAD_INSTALL_DIR}/"
cp "${VALIDATED_CONFIG}" "${PAYLOAD_INSTALL_DIR}/config.json"
chmod 755 \
  "${PAYLOAD_ROOT}/Library" \
  "${PAYLOAD_ROOT}/Library/Application Support" \
  "${PAYLOAD_ROOT}/Library/Application Support/TestFlying" \
  "${PAYLOAD_INSTALL_DIR}"
chmod 755 "${PAYLOAD_INSTALL_DIR}/testflying-build-runner" "${PAYLOAD_INSTALL_DIR}/package-agent"
chmod 600 "${PAYLOAD_INSTALL_DIR}/config.json"
cp "${PACKAGING_DIR}/postinstall" "${PACKAGE_SCRIPTS}/postinstall"
chmod 755 "${PACKAGE_SCRIPTS}/postinstall"

PACKAGE_NAME="TestFlyingBuildRunner-${VERSION}-darwin-${ARCH}.pkg"
pkgbuild \
  --root "${PAYLOAD_ROOT}" \
  --scripts "${PACKAGE_SCRIPTS}" \
  --identifier com.testflying.build-runner \
  --version "${VERSION}" \
  --install-location / \
  "${FINAL_DIR}/${PACKAGE_NAME}"

mkdir -p "$(dirname "${OUTPUT_DIR}")"
if [[ -e "${OUTPUT_DIR}" ]]; then
  echo "OUTPUT_DIR was created while packaging: ${OUTPUT_DIR}" >&2
  exit 2
fi
mv "${FINAL_DIR}" "${OUTPUT_DIR}"
printf 'Build runner installer created at %s\n' "${OUTPUT_DIR}"
