# Testflying Screenshot Connector for macOS

This ZIP contains a universal Rust worker and the credentials for one Screenshot Connector. Keep it private. Move the extracted directory to a permanent location under `~/Library/Application Support/` on the assigned Mac.

1. Set owner-only permissions with `chmod 700 . && chmod 600 connector.json`.
2. Sign in to Codex with the dedicated macOS account.
3. Edit `connector.json` on this Mac. Set the absolute paths for `codexPath` and `codexHome`. Define the named `codexPermissionProfile` in the machine's Codex configuration. Grant read access to the configured project and SDK, write access to the worker's private temporary directory, and only the simulator access this account needs.
4. Prepare the local projects and simulators. Optionally set `projectRoots` to the absolute directories Codex should search. The default is the screenshot account's home directory plus project roots in existing `profiles`, which are optional hints. Android uses a local ADB server and an ADB binary from `adbPath` or the Android SDK.
5. Run `sh ./install.sh` as the screenshot account and approve the administrator prompt. The script starts a system LaunchDaemon that runs as the screenshot account, reconnects to the center after network interruption, and starts at macOS boot. Keep the extracted directory in place.
6. Open the admin page and refresh the screenshot node connection status. The installer also prints the exact `sudo launchctl print system/…` command for local status.

`sh ./run.sh` runs a single job manually. The background service receives task notifications over the encrypted WSS connection. Every task starts a fresh Codex session to find the project and prepare its screen on a simulator. The Rust worker validates the selected project and device before capturing.

To upgrade an existing node, copy the new `screenshot-connector`, `run.sh`, `install.sh` and `testflying-prod-ca.crt` into the original installation directory. Keep its `connector.json` and run `sh ./install.sh` to restart the service.

Example project search configuration

```json
{
  "projectRoots": ["/absolute/path/to/projects"],
  "adbPath": "/absolute/path/to/adb"
}
```

The bundled CA is used for the center connection. For a different private CA, set `caFile` to its absolute path.
