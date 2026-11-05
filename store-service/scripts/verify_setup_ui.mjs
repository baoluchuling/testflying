import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import http from "node:http";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";
import { generateKeyPairSync, createHash, randomUUID } from "node:crypto";

const project = fs.realpathSync(path.resolve(path.dirname(fileURLToPath(import.meta.url)), ".."));
const root = process.env.ARTIFACT_RUN_ROOT;
assert(root && path.isAbsolute(root) && fs.realpathSync(root) === root);
assert(!root.startsWith(project + path.sep) && !project.startsWith(root + path.sep));
const marker = path.join(root, ".artifact-lifecycle-owner.json");
assert.equal(process.env.ARTIFACT_RUN_OWNERSHIP_MARKER, marker);
const fd = fs.openSync(marker, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW);
try {
  const before = fs.fstatSync(fd);
  assert(before.isFile() && before.uid === process.getuid() && before.nlink === 1 && !(before.mode & 0o222) && before.size <= 4096);
  const actual = JSON.parse(fs.readFileSync(fd, "utf8"));
  assert.deepEqual(actual, { schema_version: 1, owner: "codex-artifact-lifecycle", run_id: path.basename(root), project_root: project });
  const after = fs.lstatSync(marker);
  assert.equal(before.ino, after.ino);
  assert.equal(before.dev, after.dev);
  assert.equal(before.mtimeMs, after.mtimeMs);
} finally { fs.closeSync(fd); }

const tmp = path.join(root, "tmp");
const require = createRequire(path.join(tmp, "browser/package.json"));
const { chromium } = require("playwright-core");
const JSZip = require(path.join(project, "crates/store-api/src/setup/vendor/jszip.min.js"));
const web = path.join(project, "crates/store-api/src/setup");
const token = process.env.SETUP_ADMIN_TOKEN || "browser-admin-test-0123456789";
const production = !!process.env.SETUP_BASE_URL;
const nodes = [];
const created = new Set();
const posts = [];
const violations = [];
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
let server;
let base = process.env.SETUP_BASE_URL;
let downloadAttempts = 0;
const executable = fs.readFileSync(path.join(project, "releases/windows-amd64/store-connector.exe"));

if (!production) {
  const assetPaths = {
    "": [path.join(web, "index.html"), "text/html"],
    "app.mjs": [path.join(web, "app.mjs"), "text/javascript"],
    "packager.mjs": [path.join(web, "packager.mjs"), "text/javascript"],
    "style.css": [path.join(web, "style.css"), "text/css"],
    "jszip.min.js": [path.join(web, "vendor/jszip.min.js"), "text/javascript"],
    "license.txt": [path.join(web, "vendor/JSZIP-LICENSE.markdown"), "text/plain"],
    "ca.crt": [path.join(project, "deploy/testflying-prod-ca.crt"), "application/x-pem-file"],
    "start.cmd": [path.join(project, "packaging/connector/start.cmd"), "text/plain"],
    "start.sh": [path.join(project, "packaging/connector/start.sh"), "text/plain"],
  };
  const csp = fs.readFileSync(path.join(project, "crates/store-api/src/setup.rs"), "utf8").match(/const CSP: &str = "([^"]+)"/)[1];
  server = http.createServer(async (req, res) => {
    try {
      const url = new URL(req.url, "http://localhost");
      res.setHeader("Cache-Control", "no-store");
      const relative = url.pathname.replace(/^\/store-api\/connector-setup\//, "");
      if (url.pathname.startsWith("/store-api/connector-setup/") && Object.hasOwn(assetPaths, relative)) {
        const [file, type] = assetPaths[relative];
        res.setHeader("Content-Security-Policy", csp);
        res.setHeader("Content-Type", type);
        res.end(fs.readFileSync(file));
        return;
      }
      if (url.pathname === "/favicon.ico") { res.writeHead(204).end(); return; }
      res.setHeader("Content-Type", "application/json");
      if (req.headers.authorization !== `Bearer ${token}`) { res.writeHead(401).end("{}"); return; }
      if (url.pathname === "/store-api/v1/connectors" && req.method === "GET") {
        res.end(JSON.stringify({ connectors: nodes })); return;
      }
      if (url.pathname === "/store-api/v1/connectors" && req.method === "POST") {
        let body = "";
        for await (const chunk of req) body += chunk;
        const data = JSON.parse(body);
        posts.push(data);
        const node = { connectorId: randomUUID(), name: data.name, enabled: true, connected: false, appCount: 0 };
        nodes.push(node);
        res.writeHead(201).end(JSON.stringify({ connector: node })); return;
      }
      const match = /^\/store-api\/v1\/connectors\/([^/]+)\/installer\/windows-amd64$/.exec(url.pathname);
      if (match) {
        downloadAttempts++;
        if (downloadAttempts === 1) { res.writeHead(503).end("{}"); return; }
        const zip = new JSZip();
        zip.file("store-connector.exe", executable);
        zip.file("start.cmd", "original launcher");
        zip.file("connector.json", JSON.stringify({ connectorId: match[1], token: "browser-node-test-token",
          serverUrl: "https://store.example.invalid/store-api", messageKey: "browser-message-key", uploadKey: "browser-upload-key",
          baseUrl: null, listenAddr: "0.0.0.0:8100", requestTimeoutSeconds: 60, provider: { mode: "live" } }));
        res.setHeader("Content-Type", "application/zip");
        res.end(await zip.generateAsync({ type: "nodebuffer" })); return;
      }
      res.writeHead(404).end("{}");
    } catch { res.writeHead(500).end("{}"); }
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  base = `http://127.0.0.1:${server.address().port}/store-api`;
}

const origin = new URL(base).origin;
const privateKey = generateKeyPairSync("ec", { namedCurve: "prime256v1" }).privateKey.export({ type: "pkcs8", format: "pem" });
const rsa = generateKeyPairSync("rsa", { modulusLength: 2048 }).privateKey.export({ type: "pkcs8", format: "pem" });
const appleFile = path.join(tmp, "AuthKey_TESTKEY123.p8");
const googleFile = path.join(tmp, "service-account.json");
fs.writeFileSync(appleFile, privateKey, { mode: 0o600, flag: "wx" });
fs.writeFileSync(googleFile, JSON.stringify({ type: "service_account", client_email: "test@example.invalid", private_key: rsa }), { mode: 0o600, flag: "wx" });
const checks = [];
let browser;
let reportPath;
try {
  browser = await chromium.connectOverCDP(process.env.SETUP_CDP_ENDPOINT);
  const context = await browser.newContext({ acceptDownloads: true, viewport: { width: 1440, height: 1100 } });
  const page = await context.newPage();
  page.on("pageerror", () => violations.push("uncaught browser error"));
  page.on("console", (message) => { if (/Content Security Policy|Refused to execute|Refused to load/.test(message.text())) violations.push("CSP violation"); });
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (["http:", "https:"].includes(url.protocol) && url.origin !== origin) violations.push("external network request");
    const body = request.postData() || "";
    if (body.includes("BEGIN PRIVATE KEY") || body.includes("private_key") || body.includes("test-issuer")) violations.push("credentials sent to center");
  });
  page.on("response", async (response) => {
    if (response.request().method() === "POST" && new URL(response.url()).pathname.endsWith("/v1/connectors") && response.status() === 201) {
      const value = await response.json(); created.add(value.connector.connectorId);
    }
  });
  await page.goto(base + "/connector-setup/", { waitUntil: "networkidle" });
  await page.locator("#adminToken").fill("invalid-token");
  await page.locator("#loginButton").click();
  await page.locator("#status[data-kind=error]").waitFor();
  checks.push("invalid administrator token rejected");
  await page.locator("#adminToken").fill(token);
  await page.locator("#loginButton").click();
  await page.locator("#setupForm").waitFor({ state: "visible" });
  assert.equal(await page.locator("#adminToken").inputValue(), "");
  assert.deepEqual(await page.evaluate(() => {
    const form = document.getElementById("setupForm");
    return [typeof form.nodeName, typeof form.target];
  }), ["string", "string"]);
  checks.push("administrator login clears the token field");
  if (!production) await page.screenshot({ path: path.join(tmp, "setup-preview.png"), fullPage: true });
  await page.locator("#connectorName").fill("Browser verification · dual platform");
  await page.locator("#issuerId").fill("test-issuer");
  await page.locator("#appleFile").setInputFiles(appleFile);
  await page.locator("#googleFile").setInputFiles(googleFile);
  assert.equal(await page.locator("#keyId").inputValue(), "TESTKEY123");
  await page.locator("#appleFile").setInputFiles({ name: "AuthKey_SECONDKEY0.p8", mimeType: "application/octet-stream", buffer: Buffer.from(privateKey) });
  assert.equal(await page.locator("#keyId").inputValue(), "SECONDKEY0");
  checks.push("Apple Key ID recognized from the original filename");
  if (!production) {
    await page.locator("#downloadButton").click();
    await page.locator("#status[data-kind=error]").waitFor();
    assert.equal(nodes.length, 1);
    assert.notEqual(await page.locator("#connector").inputValue(), "new");
    checks.push("failed download retains the newly created node");
  }
  const [download] = await Promise.all([page.waitForEvent("download"), page.locator("#downloadButton").click()]);
  const downloaded = path.join(tmp, "dual-platform.zip");
  await download.saveAs(downloaded);
  const zip = await JSZip.loadAsync(fs.readFileSync(downloaded), { checkCRC32: true });
  const config = JSON.parse(await zip.file("connector.json").async("string"));
  assert.equal(config.connectorId, await page.locator("#connector").inputValue());
  assert.equal(config.provider.applePrivateKeyPath, "credentials/apple-private-key.p8");
  assert.equal(config.provider.appleKeyId, "SECONDKEY0");
  assert.equal(config.provider.googleServiceAccountPath, "credentials/google-service-account.json");
  assert.equal(hash(await zip.file("store-connector.exe").async("nodebuffer")), hash(executable));
  assert.equal(hash(await zip.file(config.provider.applePrivateKeyPath).async("nodebuffer")), hash(privateKey));
  assert.equal(config.apps, undefined);
  assert.equal(config.bundleIdentifier, undefined);
  assert.match(await zip.file("start.cmd").async("string"), /SSL_CERT_FILE/);
  assert(zip.file("testflying-prod-ca.crt"));
  checks.push("real Windows executable preserved in a downloadable dual-platform private ZIP");
  checks.push("both credential paths are automatic and no application list is included");
  if (!production) assert.equal(nodes.length, 1);
  const firstId = config.connectorId;
  await page.locator("#appleEnabled").uncheck();
  const [second] = await Promise.all([page.waitForEvent("download"), page.locator("#downloadButton").click()]);
  const single = path.join(tmp, "google-only.zip");
  await second.saveAs(single);
  const singleZip = await JSZip.loadAsync(fs.readFileSync(single));
  assert.equal(JSON.parse(await singleZip.file("connector.json").async("string")).connectorId, firstId);
  assert.equal(singleZip.file("credentials/apple-private-key.p8"), null);
  checks.push("repeat download reuses node identity and omits disabled-platform secrets");
  await page.locator("#logout").click();
  await page.locator("#loginPanel").waitFor({ state: "visible" });
  assert.equal(await page.locator("#appleFile").inputValue(), "");
  assert.equal(await page.locator("#googleFile").inputValue(), "");
  assert.deepEqual(await page.evaluate(() => [localStorage.length, sessionStorage.length]), [0, 0]);
  checks.push("logout clears files and browser storage remains empty");
  await page.setViewportSize({ width: 390, height: 844 });
  assert(await page.evaluate(() => document.body.scrollWidth <= window.innerWidth));
  checks.push("mobile layout has no horizontal overflow");
  assert.deepEqual(violations, []);
  if (!production) for (const post of posts) assert.deepEqual(Object.keys(post), ["name"]);
  checks.push("no credential file or account fields are uploaded and CSP passes");
  reportPath = path.join(project, "docs", production ? "setup-deployment-verification.json" : "setup-ui-verification.json");
  const report = fs.existsSync(reportPath) ? JSON.parse(fs.readFileSync(reportPath, "utf8")) : {};
  if (production && report.stage === "api-deployed") {
    report.apiChecks = report.checks;
    report.apiRunId = report.runId;
  }
  Object.assign(report, { runId: path.basename(root), stage: "browser-tests-passed", browserVerified: false, production,
    checks, windowsExecutableSha256: hash(executable), credentialFilesUploaded: false,
    verificationNodeIds: [...created],
    remainingTemporaryProcesses: [], cleanup: "See the supervisor cleanup report for this runId" });
  fs.writeFileSync(reportPath, JSON.stringify(report, null, 2) + "\n");
  await context.close();
} finally {
  let cleanupFailed = false;
  try {
    if (production) for (const id of created) {
      try {
        const response = await fetch(base + "/v1/connectors/" + encodeURIComponent(id), { method: "PATCH", headers: {
          Authorization: `Bearer ${token}`, "Content-Type": "application/json" }, body: JSON.stringify({ enabled: false }) });
        if (response.status !== 200) cleanupFailed = true;
      } catch { cleanupFailed = true; }
    }
  } finally {
    if (browser) await browser.close();
    if (server) await new Promise((resolve) => server.close(resolve));
  }
  if (cleanupFailed) throw new Error("Verification node cleanup incomplete; inspect the report");
}
if (reportPath) {
  const report = JSON.parse(fs.readFileSync(reportPath, "utf8"));
  Object.assign(report, { stage: "browser-verified", browserVerified: true, verificationNodesDisabled: production });
  fs.writeFileSync(reportPath, JSON.stringify(report, null, 2) + "\n");
  if (!production) fs.copyFileSync(path.join(tmp, "setup-preview.png"), path.join(project, "docs/connector-setup-preview.png"));
}
