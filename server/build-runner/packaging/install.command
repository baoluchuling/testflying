#!/usr/bin/env bash
set -euo pipefail

PACKAGE_DIR="$(cd "$(dirname "$0")" && pwd)"
SYSTEM_INSTALL_ROOT="/Library/Application Support/TestFlying/build-runner"
USER_INSTALL_ROOT="${HOME}/Library/Application Support/TestFlying/build-runner"
LOG_ROOT="${HOME}/Library/Logs/TestFlying/build-runner"
PLIST="${HOME}/Library/LaunchAgents/com.testflying.build-runner.plist"
INSTALL_ROOT="${SYSTEM_INSTALL_ROOT}"

if [[ ! -w "$(dirname "${SYSTEM_INSTALL_ROOT}")" ]] && [[ ! -d "${SYSTEM_INSTALL_ROOT}" || ! -w "${SYSTEM_INSTALL_ROOT}" ]]; then
  INSTALL_ROOT="${USER_INSTALL_ROOT}"
fi

mkdir -p "${INSTALL_ROOT}" "${LOG_ROOT}" "$(dirname "${PLIST}")"
cp "${PACKAGE_DIR}/testflying-build-runner" "${INSTALL_ROOT}/testflying-build-runner"
if [[ ! -x "${PACKAGE_DIR}/package-agent" ]]; then
  echo "Missing bundled package-agent at ${PACKAGE_DIR}/package-agent" >&2
  exit 1
fi
cp "${PACKAGE_DIR}/package-agent" "${INSTALL_ROOT}/package-agent"
cp "${PACKAGE_DIR}/config.json" "${INSTALL_ROOT}/config.json"
chmod +x "${INSTALL_ROOT}/testflying-build-runner" "${INSTALL_ROOT}/package-agent"
chmod 600 "${INSTALL_ROOT}/config.json"

/usr/bin/python3 - "${INSTALL_ROOT}/config.json" "${INSTALL_ROOT}/package-agent" <<'PY'
import json
import os
import sys
from pathlib import Path

path = Path(sys.argv[1])
config = json.loads(path.read_text(encoding="utf-8"))
if not isinstance(config, dict):
    raise SystemExit("config.json must be an object")
config["packageAgentBin"] = sys.argv[2]
temporary = path.with_suffix(".json.tmp")
temporary.write_text(json.dumps(config, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
os.replace(temporary, path)
PY

cat > "${INSTALL_ROOT}/run-build-runner.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

INSTALL_ROOT="${1:?install root is required}"
CONFIG_PATH="${INSTALL_ROOT}/config.json"
BINARY_PATH="${INSTALL_ROOT}/testflying-build-runner"
PACKAGE_AGENT_PATH="${INSTALL_ROOT}/package-agent"

if [[ ! -f "${CONFIG_PATH}" ]]; then
  echo "Missing config.json at ${CONFIG_PATH}" >&2
  exit 1
fi

if [[ ! -x "${BINARY_PATH}" ]]; then
  echo "Missing runner binary at ${BINARY_PATH}" >&2
  exit 1
fi

if [[ ! -x "${PACKAGE_AGENT_PATH}" ]]; then
  echo "Missing package-agent at ${PACKAGE_AGENT_PATH}" >&2
  exit 1
fi

eval "$(
  /usr/bin/python3 - "${CONFIG_PATH}" <<'PY'
import json
import shlex
import sys

config_path = sys.argv[1]
with open(config_path, "r", encoding="utf-8") as handle:
    config = json.load(handle)

required = {
    "TESTFLYING_BUILD_RUNNER_ID": "runnerId",
    "TESTFLYING_BUILD_RUNNER_NAME": "name",
    "TESTFLYING_BUILD_RUNNER_TOKEN": "token",
    "TESTFLYING_BUILD_RUNNER_SERVER_URL": "serverUrl",
    "TESTFLYING_BUILD_RUNNER_ROOT": "rootDir",
    "TESTFLYING_PACKAGE_AGENT_BIN": "packageAgentBin",
}
optional = {
    "TESTFLYING_BUILD_RUNNER_VERSION": "version",
    "TESTFLYING_PACKAGE_AGENT_VERSION": "packageAgentVersion",
}
csv_fields = {
    "TESTFLYING_BUILD_RUNNER_LABELS": "labels",
    "TESTFLYING_BUILD_RUNNER_LLM_ADAPTERS": "llmAdapters",
}

lines = []
for env_name, key in required.items():
    value = str(config.get(key, "")).strip()
    if not value:
        raise SystemExit(f"config.json missing required field: {key}")
    lines.append(f"export {env_name}={shlex.quote(value)}")

for env_name, key in optional.items():
    value = str(config.get(key, "")).strip()
    if value:
        lines.append(f"export {env_name}={shlex.quote(value)}")

capacity = int(config.get("capacity", 1))
if capacity != 1:
    raise SystemExit("config.json capacity must be 1 in the current implementation")

for env_name, key in csv_fields.items():
    values = config.get(key, [])
    if not isinstance(values, list):
        raise SystemExit(f"config.json field {key} must be a list")
    joined = ",".join(str(item).strip() for item in values if str(item).strip())
    if joined:
        lines.append(f"export {env_name}={shlex.quote(joined)}")

print("\n".join(lines))
PY
)"

exec "${BINARY_PATH}"
EOF
chmod +x "${INSTALL_ROOT}/run-build-runner.sh"

cat > "${PLIST}" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>com.testflying.build-runner</string>
  <key>ProgramArguments</key>
  <array>
    <string>${INSTALL_ROOT}/run-build-runner.sh</string>
    <string>${INSTALL_ROOT}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>ThrottleInterval</key>
  <integer>10</integer>
  <key>WorkingDirectory</key>
  <string>${INSTALL_ROOT}</string>
  <key>StandardOutPath</key>
  <string>${LOG_ROOT}/runner.log</string>
  <key>StandardErrorPath</key>
  <string>${LOG_ROOT}/runner.err.log</string>
</dict>
</plist>
PLIST

launchctl bootout "gui/$(id -u)" "${PLIST}" >/dev/null 2>&1 || true
launchctl bootstrap "gui/$(id -u)" "${PLIST}"
launchctl kickstart -k "gui/$(id -u)/com.testflying.build-runner"

printf 'Installed build runner to %s\n' "${INSTALL_ROOT}"
printf 'LaunchAgent loaded: %s\n' "${PLIST}"
