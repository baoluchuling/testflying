import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { generateKeyPairSync, createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { buildInstaller, validateAccount, CREDENTIAL_LIMIT } from "../src/setup/packager.mjs";

const require = createRequire(import.meta.url);
const JSZip = require("../src/setup/vendor/jszip.min.js");
const encode = (s) => new TextEncoder().encode(s);
const digest = (s) => createHash("sha256").update(s).digest("hex");
const appleKey = generateKeyPairSync("ec", { namedCurve: "prime256v1" }).privateKey.export({ type: "pkcs8", format: "pem" });
const googleKey = generateKeyPairSync("rsa", { modulusLength: 2048 }).privateKey.export({ type: "pkcs8", format: "pem" });
const account = {
  apple: { issuerId: "test-issuer", keyId: "TESTKEY123", content: encode(appleKey) },
  google: { content: encode(JSON.stringify({ type: "service_account", client_email: "test@example.invalid", private_key: googleKey })) },
};
const config = { connectorId: "12345678-test-node", token: "node-test-token", serverUrl: "https://store.example.invalid/store-api",
  messageKey: "message-test-key", uploadKey: "upload-test-key", baseUrl: null, listenAddr: "0.0.0.0:8100", requestTimeoutSeconds: 60,
  provider: { mode: "live" }, apps: ["must-not-be-copied"] };
const assets = {
  caBytes: new Uint8Array(await readFile(new URL("../../../deploy/testflying-prod-ca.crt", import.meta.url))),
  scripts: Object.fromEntries(await Promise.all(["start.cmd", "start.sh", "install.cmd", "install.ps1", "install.sh"].map(async (name) =>
    [name, await readFile(new URL(`../../../packaging/connector/${name}`, import.meta.url), "utf8")]))),
};
const executable = new Uint8Array([77, 90, 1, 2, 3, 4, 5]);
async function base(target = "windows-amd64", extra = {}, override = {}) {
  const zip = new JSZip();
  zip.file(target === "windows-amd64" ? "store-connector.exe" : "store-connector", executable);
  zip.file(target === "windows-amd64" ? "start.cmd" : "start.sh", "original launcher");
  zip.file("connector.json", JSON.stringify({ ...config, ...override }));
  for (const [path, value] of Object.entries(extra)) zip.file(path, value);
  return zip.generateAsync({ type: "uint8array" });
}

test("one Windows node contains both accounts, safe relative paths and unchanged executable", async () => {
  const result = await buildInstaller(await base(), { target: "windows-amd64", account }, assets, JSZip);
  const zip = await JSZip.loadAsync(result.bytes, { checkCRC32: true });
  const actual = JSON.parse(await zip.file("connector.json").async("string"));
  for (const key of ["connectorId", "token", "serverUrl", "messageKey", "uploadKey", "listenAddr", "baseUrl"]) assert.equal(actual[key], config[key]);
  assert.equal(actual.provider.applePrivateKeyPath, "credentials/apple-private-key.p8");
  assert.equal(actual.provider.googleServiceAccountPath, "credentials/google-service-account.json");
  assert.equal(actual.provider.appleIssuerId, "test-issuer");
  assert.equal(digest(await zip.file(actual.provider.applePrivateKeyPath).async("string")), digest(appleKey));
  assert.equal(digest(JSON.parse(await zip.file(actual.provider.googleServiceAccountPath).async("string")).private_key), digest(googleKey));
  assert.deepEqual(await zip.file("store-connector.exe").async("uint8array"), executable);
  assert.equal(actual.apps, undefined);
  assert.equal(actual.bundleIdentifier, undefined);
  assert.match(await zip.file("start.cmd").async("string"), /cd \/d "%~dp0"/);
  assert.match(await zip.file("start.cmd").async("string"), /SSL_CERT_FILE/);
  assert.match(await zip.file("install.cmd").async("string"), /install\.ps1/);
  assert.match(await zip.file("install.ps1").async("string"), /New-ScheduledTaskTrigger -AtStartup/);
  assert.ok(!(await zip.file("start.cmd").async("string")).replace(/\r\n/g, "").includes("\n"));
  assert.ok(zip.file("testflying-prod-ca.crt"));
  assert.equal(zip.file("connector.json").unixPermissions & 0o777, 0o600);
  assert.equal(zip.file(actual.provider.applePrivateKeyPath).unixPermissions & 0o777, 0o600);
  assert.equal(result.fileName, "connector-windows-amd64-12345678.zip");
});

test("Apple-only configuration does not bundle or retain Google credentials", async () => {
  const result = await buildInstaller(await base(), { target: "windows-amd64", account: { apple: account.apple } }, assets, JSZip);
  const zip = await JSZip.loadAsync(result.bytes);
  const actual = JSON.parse(await zip.file("connector.json").async("string"));
  assert.equal(actual.provider.googleServiceAccountPath, undefined);
  assert.equal(zip.file("credentials/google-service-account.json"), null);
});

test("Google-only Unix package uses the same identity and a working-directory-aware launcher", async () => {
  for (const target of ["linux-amd64", "darwin-arm64"]) {
    const result = await buildInstaller(await base(target), { target, account: { google: account.google } }, assets, JSZip);
    const zip = await JSZip.loadAsync(result.bytes);
    const actual = JSON.parse(await zip.file("connector.json").async("string"));
    assert.equal(actual.connectorId, config.connectorId);
    assert.equal(actual.provider.applePrivateKeyPath, undefined);
    assert.equal(zip.file("credentials/apple-private-key.p8"), null);
    assert.match(await zip.file("start.sh").async("string"), /CDPATH=/);
    assert.equal(zip.file("start.sh").unixPermissions & 0o777, 0o700);
    assert.equal(!!zip.file("install.sh"), target === "darwin-arm64");
  }
});

test("program-only upgrades leave configuration and shop credentials out of the ZIP", async () => {
  for (const target of ["windows-amd64", "darwin-arm64"]) {
    const result = await buildInstaller(await base(target), { target, upgrade: true }, assets, JSZip);
    const zip = await JSZip.loadAsync(result.bytes, { checkCRC32: true });
    const entry = target === "windows-amd64" ? "install.cmd" : "install.sh";
    assert.ok(zip.file(entry));
    assert.equal((await zip.file("node-id.txt").async("string")).trim(), config.connectorId);
    assert.equal(zip.file("connector.json"), null);
    assert(!Object.keys(zip.files).some((name) => name.startsWith("credentials/")));
    assert.match(result.fileName, /-upgrade\.zip$/);
  }
  await assert.rejects(buildInstaller(await base("linux-amd64"), { target: "linux-amd64", upgrade: true }, assets, JSZip), /不支持/);
});

test("invalid or oversized credentials fail before any installation work", () => {
  assert.throws(() => validateAccount({}), /至少启用/);
  assert.throws(() => validateAccount({ apple: { ...account.apple, issuerId: "" } }), /Issuer ID/);
  assert.throws(() => validateAccount({ apple: { ...account.apple, content: encode("not a key") } }), /PEM/);
  assert.throws(() => validateAccount({ google: { content: encode("not json") } }), /JSON/);
  assert.throws(() => validateAccount({ google: { content: encode("{}") } }), /service_account/);
  assert.throws(() => validateAccount({ google: { content: new Uint8Array(CREDENTIAL_LIMIT + 1) } }), /256 KiB/);
});

test("archive rejects injected files, unsafe paths and invalid center identity", async () => {
  await assert.rejects(buildInstaller(await base("windows-amd64", { "extra.exe": "unexpected" }), { target: "windows-amd64", account }, assets, JSZip), /不允许/);
  await assert.rejects(buildInstaller(await base("windows-amd64", { "../leaked.txt": "unexpected" }), { target: "windows-amd64", account }, assets, JSZip), /不允许/);
  await assert.rejects(buildInstaller(await base("windows-amd64", {}, { token: "" }), { target: "windows-amd64", account }, assets, JSZip), /配置不完整/);
});

test("missing launch assets or unsupported targets never produce a misleading package", async () => {
  await assert.rejects(buildInstaller(await base(), { target: "windows-arm64", account }, assets, JSZip), /不支持/);
  await assert.rejects(buildInstaller(await base(), { target: "windows-amd64", account }, { ...assets, caBytes: encode("invalid") }, JSZip), /CA/);
  await assert.rejects(buildInstaller(await base(), { target: "windows-amd64", account },
    { ...assets, scripts: { ...assets.scripts, "start.cmd": "invalid" } }, JSZip), /启动脚本/);
});
