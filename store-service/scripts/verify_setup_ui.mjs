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
const token = process.env.SETUP_ADMIN_TOKEN;
const production = !!process.env.SETUP_BASE_URL;
const nodes = [];
const integrationFixtures = [];
const fixtureAdminToken = "fixture-admin-token";
const fixtureIntegrationToken = "fixture-integration-token";
const created = new Set();
const posts = [];
const violations = [];
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
let server;
let base = process.env.SETUP_BASE_URL;
let downloadAttempts = 0;
let listAttempts = 0;
const executable = fs.readFileSync(path.join(project, "releases/windows-amd64/store-connector.exe"));

if (!production) {
  const assetPaths = {
    "": [path.join(web, "index.html"), "text/html"],
    "app.mjs": [path.join(web, "app.mjs"), "text/javascript"],
    "integrations.mjs": [path.join(web, "integrations.mjs"), "text/javascript"],
    "packager.mjs": [path.join(web, "packager.mjs"), "text/javascript"],
    "style.css": [path.join(web, "style.css"), "text/css"],
    "jszip.min.js": [path.join(web, "vendor/jszip.min.js"), "text/javascript"],
    "license.txt": [path.join(web, "vendor/JSZIP-LICENSE.markdown"), "text/plain"],
    "ca.crt": [path.join(project, "deploy/testflying-prod-ca.crt"), "application/x-pem-file"],
    "start.cmd": [path.join(project, "packaging/connector/start.cmd"), "text/plain"],
    "start.sh": [path.join(project, "packaging/connector/start.sh"), "text/plain"],
    "install.cmd": [path.join(project, "packaging/connector/install.cmd"), "text/plain"],
    "install.ps1": [path.join(project, "packaging/connector/install.ps1"), "text/plain"],
    "install.sh": [path.join(project, "packaging/connector/install.sh"), "text/plain"],
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
      if (url.pathname.startsWith("/store-api/v1/integrations")) {
        if (req.headers.authorization !== `Bearer ${fixtureAdminToken}`) { res.writeHead(401).end("{}"); return; }
        if (req.method === "POST") {
          let body = "";
          for await (const chunk of req) body += chunk;
          const integration = { id: randomUUID(), ...JSON.parse(body) };
          integrationFixtures.push(integration);
          res.writeHead(201).end(JSON.stringify({ integration, token: fixtureIntegrationToken }));
        } else if (req.method === "DELETE") {
          integrationFixtures.pop();
          res.writeHead(204).end();
        } else if (url.pathname.endsWith("/token")) {
          res.end(JSON.stringify({ token: fixtureIntegrationToken }));
        } else {
          res.end(JSON.stringify({ integrations: integrationFixtures }));
        }
        return;
      }
      if (url.pathname === "/store-api/connector-setup/api/connectors" && req.method === "GET") {
        if (++listAttempts === 1) { res.writeHead(503).end("{}"); return; }
        res.end(JSON.stringify({ connectors: nodes })); return;
      }
      if (url.pathname === "/store-api/connector-setup/api/connectors" && req.method === "POST") {
        let body = "";
        for await (const chunk of req) body += chunk;
        const data = JSON.parse(body);
        posts.push(data);
        const node = { connectorId: randomUUID(), name: data.name, enabled: true, connected: false, appCount: 0 };
        nodes.push(node);
        res.writeHead(201).end(JSON.stringify({ connector: node })); return;
      }
      const match = /^\/store-api\/connector-setup\/api\/connectors\/([^/]+)\/installer\/windows-amd64$/.exec(url.pathname);
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
const navigation = new URL(base + "/connector-setup/");
let credentialsEntered = false;
let secureNavigationProbes = 0;
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
  if (!production) {
    await page.addInitScript(() => {
      Object.defineProperty(navigator, "clipboard", {
        value: { writeText: async (value) => { window.fixtureCopiedToken = value; } },
      });
    });
  }
  const requestOutcomes = [];
  function recordOutcome(request, outcome) {
    const route = new URL(request.url()).pathname;
    if (!route.includes("/connector-setup/")) return;
    const operation = route.endsWith("/api/connectors") ? "connectors"
      : route.includes("/installer/") ? "installer" : route.split("/").at(-1) || "page";
    requestOutcomes.push([request.method(), operation, outcome]);
  }
  async function downloadPackage() {
    try {
      const [download] = await Promise.all([page.waitForEvent("download"), page.locator("#downloadButton").click()]);
      return download;
    } catch {
      const state = await page.locator("#status").textContent({ timeout: 2000 });
      // Exclude server response bodies and generated configuration.
      throw new Error(JSON.stringify({ stage: state?.split("\n")[0], requests: requestOutcomes, violations }));
    }
  }
  page.on("requestfailed", (request) => recordOutcome(request, request.failure()?.errorText));
  page.on("pageerror", () => violations.push("uncaught browser error"));
  page.on("console", (message) => { if (/Content Security Policy|Refused to execute|Refused to load/.test(message.text())) violations.push("CSP violation"); });
  await page.route("**/*", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const body = request.postData() || "";
    const authenticated = !!request.headers().authorization;
    const fixtureAdminRequest = !production && url.origin === origin && url.pathname.startsWith("/store-api/v1/integrations");
    if (authenticated && !fixtureAdminRequest) {
      violations.push("browser sent an Authorization header");
      return route.abort();
    }
    if (["http:", "https:"].includes(url.protocol) && url.origin !== origin) {
      // Chrome can probe HTTPS before falling back to the requested HTTP page.
      // This is permitted only for a credential-free initial navigation to this exact center page.
      if (production && !credentialsEntered && !authenticated && !body && request.isNavigationRequest() &&
          request.method() === "GET" && url.protocol === "https:" && url.hostname === navigation.hostname &&
          !url.port && url.pathname === navigation.pathname && !url.search) {
        secureNavigationProbes++;
      } else {
        violations.push({ kind: "blocked cross-origin request", sameHost: url.hostname === navigation.hostname,
          scheme: url.protocol, method: request.method(), type: request.resourceType(),
          setupPath: url.pathname === navigation.pathname, authenticated, hasBody: !!body });
        return route.abort();
      }
    }
    if (body.includes("BEGIN PRIVATE KEY") || body.includes("private_key") || body.includes("test-issuer")) {
      violations.push("blocked credential file transmission");
      return route.abort();
    }
    return route.continue();
  });
  page.on("response", async (response) => {
    recordOutcome(response.request(), response.status());
    if (response.request().method() === "POST" && new URL(response.url()).pathname.endsWith("/connector-setup/api/connectors") && response.status() === 201) {
      const value = await response.json(); created.add(value.connector.connectorId);
    }
  });
  await page.goto(base + "/connector-setup/", { waitUntil: "domcontentloaded" });
  assert.equal(new URL(page.url()).origin === origin, true, "page opened on the requested origin");
  if (!production) {
    await page.locator("#status[data-kind=error]").waitFor();
    await page.locator("#retryConnection").click();
    checks.push("retry connects after an initial request failure");
  }
  await page.locator("#setupForm").waitFor({ state: "visible" });
  assert.equal(await page.locator("#setupForm input[type=password]").count(), 0);
  assert.deepEqual(await page.evaluate(() => {
    const form = document.getElementById("setupForm");
    return [typeof form.nodeName, typeof form.target];
  }), ["string", "string"]);
  checks.push("setup form loads without a login or Authorization header");
  if (!production) await page.screenshot({ path: path.join(tmp, "setup-preview.png"), fullPage: true });
  credentialsEntered = true;
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
  const download = await downloadPackage();
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
  assert.match(await zip.file("install.cmd").async("string"), /install\.ps1/);
  assert.match(await zip.file("install.ps1").async("string"), /New-ScheduledTaskTrigger -AtStartup/);
  assert(zip.file("testflying-prod-ca.crt"));
  checks.push("real Windows executable preserved in a downloadable dual-platform private ZIP");
  checks.push("both credential paths are automatic and no application list is included");
  if (!production) assert.equal(nodes.length, 1);
  const firstId = config.connectorId;
  await page.locator("#appleEnabled").uncheck();
  const second = await downloadPackage();
  const single = path.join(tmp, "google-only.zip");
  await second.saveAs(single);
  const singleZip = await JSZip.loadAsync(fs.readFileSync(single));
  assert.equal(JSON.parse(await singleZip.file("connector.json").async("string")).connectorId, firstId);
  assert.equal(singleZip.file("credentials/apple-private-key.p8"), null);
  checks.push("repeat download reuses node identity and omits disabled-platform secrets");
  await page.locator("#upgrade").check();
  assert.equal(await page.locator("#accountSection").isVisible(), false);
  const upgrade = await downloadPackage();
  const upgradePath = path.join(tmp, "program-upgrade.zip");
  await upgrade.saveAs(upgradePath);
  const upgradeZip = await JSZip.loadAsync(fs.readFileSync(upgradePath), { checkCRC32: true });
  assert.equal((await upgradeZip.file("node-id.txt").async("string")).trim(), firstId);
  assert.equal(upgradeZip.file("connector.json"), null);
  assert(!Object.keys(upgradeZip.files).some((name) => name.startsWith("credentials/")));
  assert.equal(hash(await upgradeZip.file("store-connector.exe").async("nodebuffer")), hash(executable));
  if (!production) assert.equal(nodes.length, 1);
  checks.push("program-only upgrade preserves node identity without including local configuration");
  if (!production) {
    await page.getByText("接入凭据", { exact: true }).click();
    await page.locator("#integrationAdmin").fill("wrong-fixture");
    await page.locator("#integrationUnlock button").click();
    await page.getByText("管理员 Token 无效", { exact: true }).waitFor();
    assert.equal(await page.locator("#integrationPanel").isVisible(), false);
    await page.locator("#integrationAdmin").fill(fixtureAdminToken);
    await page.locator("#integrationUnlock button").click();
    await page.locator("#integrationPanel").waitFor();
    await page.locator("#integrationName").fill("Fixture System");
    await page.locator("#integrationNodes").selectOption(firstId);
    await page.locator("#integrationIssue button").click();
    await page.getByText("Token 已发放，可复制到该系统的后端。", { exact: true }).waitFor();
    assert.equal(await page.locator("#integrationToken").inputValue(), fixtureIntegrationToken);
    assert.deepEqual(integrationFixtures[0].connectorIds, [firstId]);
    await page.locator("#integrationCopy").click();
    await page.waitForFunction((expected) => window.fixtureCopiedToken === expected, fixtureIntegrationToken);
    await page.getByText("Token 已复制。", { exact: true }).waitFor();
    await page.locator("#integrationLock").click();
    assert.equal(await page.locator("#integrationToken").inputValue(), "");
    await page.locator("#integrationAdmin").fill(fixtureAdminToken);
    await page.locator("#integrationUnlock button").click();
    await page.getByRole("button", { name: "取回 Token", exact: true }).click();
    await page.getByText("可以复制这枚接入 Token。", { exact: true }).waitFor();
    assert.equal(await page.locator("#integrationToken").inputValue(), fixtureIntegrationToken);
    page.once("dialog", (dialog) => dialog.accept());
    await page.getByRole("button", { name: "撤销", exact: true }).click();
    await page.getByText("Token 已撤销。", { exact: true }).waitFor();
    assert.equal(await page.locator("#integrationToken").inputValue(), "");
    assert.equal(integrationFixtures.length, 0);
    checks.push("admin can issue, copy, retrieve, and revoke the scoped integration token in the setup page");
  }
  credentialsEntered = false;
  await page.reload({ waitUntil: "domcontentloaded" });
  await page.locator("#setupForm").waitFor({ state: "visible" });
  assert.equal(await page.locator("#appleFile").inputValue(), "");
  assert.equal(await page.locator("#googleFile").inputValue(), "");
  assert.deepEqual(await page.evaluate(() => [localStorage.length, sessionStorage.length]), [0, 0]);
  checks.push("reload clears files and browser storage remains empty");
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
    verificationNodeIds: [...created], secureNavigationProbes,
    remainingTemporaryProcesses: [], cleanup: "See the supervisor cleanup report for this runId" });
  fs.writeFileSync(reportPath, JSON.stringify(report, null, 2) + "\n");
  await context.close();
} finally {
  let cleanupFailed = false;
  try {
    if (production) for (const id of created) {
      try {
        const response = await fetch(base.replace(/^http:/, "https:") + "/v1/connectors/" + encodeURIComponent(id), { method: "PATCH", headers: {
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
