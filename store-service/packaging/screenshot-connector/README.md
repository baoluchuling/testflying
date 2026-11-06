# Testflying Screenshot Connector for macOS

This ZIP contains a worker and the credentials for one Screenshot Connector. Keep it private and extract it to a private directory on the assigned Mac.

1. Set owner-only permissions with `chmod 700 . && chmod 600 connector.json`.
2. Install Python 3.11 or newer, activate the artifact lifecycle integration, and sign in to Codex with the dedicated macOS account.
3. Edit `connector.json` on this Mac. Set the absolute paths for `codexPath` and `codexHome`. Define the named `codexPermissionProfile` in the machine's managed Codex configuration. Grant read access to the configured project and SDK, write access to the managed work directory, and only the simulator access this account needs.
4. Add the local profiles. Use the name your callers will send and the already-running simulator's exact UUID or emulator serial. Android also requires an absolute `adbPath` and the local ADB server.
5. Run `sh ./run.sh` as the dedicated account. The program handles at most one job per invocation. Schedule that command periodically for unattended operation.

Example value for `profiles`

```json
{
  "iphone": {
    "platform": "ios",
    "projectRoot": "/absolute/path/to/ios-project",
    "deviceId": "22222222-2222-4222-8222-222222222222"
  },
  "android-tablet": {
    "platform": "android",
    "projectRoot": "/absolute/path/to/android-project",
    "deviceId": "emulator-5556",
    "adbPath": "/absolute/path/to/adb"
  }
}
```

The bundled public CA is loaded alongside system trust. For a different private CA, set `caFile` to its absolute path. Set `SCREENSHOT_CONNECTOR_PYTHON` or `SCREENSHOT_CONNECTOR_LIFECYCLE_BIN` if the tools are installed in different locations.
