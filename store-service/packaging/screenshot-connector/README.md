# Testflying Screenshot Connector for macOS

This ZIP contains the Apple Silicon (arm64) Rust worker and the credentials for one Screenshot Connector. Keep it private. Move the extracted directory to a permanent location under `~/Library/Application Support/` on the assigned Mac.

1. Set owner-only permissions with `chmod 700 . && chmod 600 connector.json`.
2. Install the Codex CLI, make sure `codex` is on the installation account's `PATH`, and sign in to Codex with that account.
3. The worker uses the account's existing Codex login and configuration with the built-in `workspace-write` sandbox. Set `codexPath` or `codexHome` in `connector.json` only to override a nonstandard installation or configuration path.
4. Prepare the local projects and simulators. Codex finds the project on this Mac using its local permissions. Android uses a local ADB server and an ADB binary from the optional `adbPath` or the Android SDK.
5. Run `sh ./install.sh` as the screenshot account and approve the administrator prompt. The script starts a system LaunchDaemon that runs as the screenshot account, reconnects to the center after network interruption, and starts at macOS boot. Keep the extracted directory in place.
6. Open the admin page and refresh the screenshot node connection status. The installer also prints the exact `sudo launchctl print system/…` command for local status.

`sh ./run.sh` runs a single job manually. The background service receives task notifications over the encrypted WSS connection. Every task starts a fresh Codex session to find the project and prepare its screen on a simulator. The Rust worker validates the selected project and device before capturing.

To upgrade an existing node, copy the new `screenshot-connector`, `run.sh`, `install.sh` and `testflying-prod-ca.crt` into the original installation directory. Keep its `connector.json` and run `sh ./install.sh` to restart the service.

From the installation directory, run `tail -F connector.log` to follow the local service log. It records connection status, job IDs and complete prompts, Codex exit status and recognized diagnostics, and screenshot delivery status. Prompts are JSON strings so multiline tasks stay on a single log line. The file is private to the installation account. The Connector removes entries older than 24 hours on startup and every minute while the service is running.

The bundled CA is used for the center connection. For a different private CA, set `caFile` to its absolute path.
