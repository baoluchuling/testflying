import { spawn } from "node:child_process";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";

const outDir = process.argv[2] || "/tmp/testflying-ui-debug/all-pages";
const baseUrl = process.argv[3] || "http://127.0.0.1:8000";
const chrome =
  process.env.CHROME_BIN || "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const adminUser = process.env.ADMIN_USER || "admin";
const adminToken = process.env.ADMIN_TOKEN || "dev-token";
const port = Number(process.env.CDP_PORT || "9337");
const authHeader = `Basic ${Buffer.from(`${adminUser}:${adminToken}`).toString("base64")}`;

const staticPages = [
  ["dashboard", "/admin"],
  ["uploads", "/admin/uploads"],
  ["apps", "/admin/apps"],
  ["accounts", "/admin/accounts"],
  ["store-reviews", "/admin/store-reviews"],
  ["api-docs", "/admin/api-docs"],
  ["builds", "/admin/builds"],
  ["devices", "/admin/devices"],
  ["app-logs", "/admin/app-logs"],
  ["notifications", "/admin/notifications"],
];

function absoluteUrl(path) {
  return new URL(path, `${baseUrl.replace(/\/+$/, "")}/`).toString();
}

async function fetchAdminJson(path) {
  const response = await fetch(absoluteUrl(path), {
    headers: { Authorization: authHeader },
  });
  if (!response.ok) {
    throw new Error(`${path} -> ${response.status}`);
  }
  return response.json();
}

async function discoverPages() {
  let accountPath = "/admin/accounts";
  let storePath = "/admin/apps";
  let reviewsPath = "/admin/store-reviews";

  try {
    const payload = await fetchAdminJson("/admin/api/developer-accounts");
    const firstAccount = Array.isArray(payload.accounts) ? payload.accounts[0] : null;
    if (firstAccount?.detailPath) {
      accountPath = firstAccount.detailPath;
    }
  } catch (error) {
    console.warn(`warn: cannot discover account page: ${error.message}`);
  }

  try {
    const payload = await fetchAdminJson("/admin/api/store-apps");
    const apps = Array.isArray(payload.apps) ? payload.apps : [];
    const storeApp = apps.find((app) => app.storeManagementPath) || null;
    if (storeApp?.storeManagementPath) {
      storePath = storeApp.storeManagementPath;
      reviewsPath = storeApp.reviewsPath || reviewsPath;
    }
  } catch (error) {
    console.warn(`warn: cannot discover store app page: ${error.message}`);
  }

  const marketingPath = storePath.endsWith("/store")
    ? storePath.replace(/\/store$/, "/marketing")
    : "/admin/apps";
  const connectionPath = storePath.endsWith("/store")
    ? storePath.replace(/\/store$/, "/connection")
    : accountPath;

  return [
    ...staticPages.slice(0, 4),
    ["account-detail", accountPath],
    ["store-detail", storePath],
    ["store-marketing", marketingPath],
    ["store-connection", connectionPath],
    ["store-reviews", reviewsPath],
    ...staticPages.slice(5),
  ];
}

mkdirSync(outDir, { recursive: true });
const profile = join(outDir, `profile-${Date.now()}`);
rmSync(profile, { recursive: true, force: true });

const child = spawn(
  chrome,
  [
    "--headless=new",
    "--disable-gpu",
    "--no-first-run",
    "--no-default-browser-check",
    "--window-size=2048,1280",
    `--user-data-dir=${profile}`,
    `--remote-debugging-port=${port}`,
    "about:blank",
  ],
  { stdio: ["ignore", "ignore", "pipe"] },
);
child.stderr.on("data", () => {});

async function waitJson(url, options) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    try {
      const response = await fetch(url, options);
      if (response.ok) return await response.json();
    } catch {
      // Wait for Chrome DevTools to become reachable.
    }
    await delay(100);
  }
  throw new Error(`timeout: ${url}`);
}

class Cdp {
  constructor(wsUrl) {
    this.ws = new WebSocket(wsUrl);
    this.id = 0;
    this.pending = new Map();
    this.events = new Map();
    this.ws.addEventListener("message", (event) => {
      const message = JSON.parse(event.data);
      if (message.id && this.pending.has(message.id)) {
        const { resolve, reject } = this.pending.get(message.id);
        this.pending.delete(message.id);
        if (message.error) reject(new Error(message.error.message));
        else resolve(message.result || {});
        return;
      }
      if (message.method && this.events.has(message.method)) {
        for (const handler of this.events.get(message.method)) handler(message.params || {});
      }
    });
  }

  async ready() {
    if (this.ws.readyState === WebSocket.OPEN) return;
    await new Promise((resolve, reject) => {
      this.ws.addEventListener("open", resolve, { once: true });
      this.ws.addEventListener("error", reject, { once: true });
    });
  }

  send(method, params = {}) {
    const id = ++this.id;
    this.ws.send(JSON.stringify({ id, method, params }));
    return new Promise((resolve, reject) => this.pending.set(id, { resolve, reject }));
  }

  waitFor(method, timeout = 8000) {
    return new Promise((resolve, reject) => {
      const handlerFn = (params) => {
        clearTimeout(timer);
        const list = this.events.get(method) || [];
        this.events.set(
          method,
          list.filter((handler) => handler !== handlerFn),
        );
        resolve(params);
      };
      const timer = setTimeout(() => {
        const list = this.events.get(method) || [];
        this.events.set(
          method,
          list.filter((handler) => handler !== handlerFn),
        );
        reject(new Error(`timeout event ${method}`));
      }, timeout);
      this.events.set(method, [...(this.events.get(method) || []), handlerFn]);
    });
  }
}

try {
  await waitJson(`http://127.0.0.1:${port}/json/version`);
  const target = await waitJson(`http://127.0.0.1:${port}/json/new?about:blank`, {
    method: "PUT",
  });
  const cdp = new Cdp(target.webSocketDebuggerUrl);
  await cdp.ready();
  await cdp.send("Page.enable");
  await cdp.send("Network.enable");
  await cdp.send("Runtime.enable");
  await cdp.send("Emulation.setDeviceMetricsOverride", {
    width: 2048,
    height: 1280,
    deviceScaleFactor: 1,
    mobile: false,
  });
  await cdp.send("Network.setExtraHTTPHeaders", {
    headers: { Authorization: authHeader },
  });

  const pages = await discoverPages();
  for (const [name, path] of pages) {
    const loaded = cdp.waitFor("Page.loadEventFired").catch(() => null);
    await cdp.send("Page.navigate", { url: absoluteUrl(path) });
    await loaded;
    await cdp.send("Runtime.evaluate", {
      expression: "document.fonts && document.fonts.ready",
      awaitPromise: true,
    });
    await delay(1300);
    const shot = await cdp.send("Page.captureScreenshot", {
      format: "png",
      captureBeyondViewport: false,
    });
    writeFileSync(join(outDir, `real-${name}.png`), Buffer.from(shot.data, "base64"));
    console.log(`captured ${name}`);
  }
  await cdp.send("Browser.close").catch(() => null);
} finally {
  child.kill("SIGTERM");
}
