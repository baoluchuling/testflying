import { CREDENTIAL_LIMIT, validateAccount, buildInstaller, installationFiles } from "./packager.mjs";
import { bindIntegrations } from "./integrations.mjs";
import { bindLogs } from "./logs.mjs";
import { bindForwarding } from "./forwarding.mjs";

const $ = (id) => document.getElementById(id);
const api = new URL("api/", window.location.href);
const setupPath = "connectors";
let session;
let nodes = [];
let automaticKeyId = "";
const integrations = bindIntegrations(request, () => $("setupForm").hidden ? undefined : session);
const logs = bindLogs(request, () => $("setupForm").hidden ? undefined : session);
const forwarding=bindForwarding(request,()=>$("setupForm").hidden?undefined:session);

function status(message, kind = "info") {
  $("status").hidden = false;
  $("status").textContent = message;
  $("status").dataset.kind = kind;
}

class Unauthorized extends Error {}

async function request(active, path, { method = "GET", body, binary = false, token } = {}) {
  active.signal.throwIfAborted();
  const response = await fetch(new URL(path, api), {
    method,
    cache: "no-store",
    credentials: "same-origin",
    mode: "same-origin",
    redirect: "error",
    referrerPolicy: "no-referrer",
    headers: {
      "X-Testflying-Setup": "1",
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
      ...(body === undefined ? {} : { "Content-Type": "application/json" }),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: active.signal,
    keepalive: path === "session" && method === "DELETE",
  });
  active.signal.throwIfAborted();
  if (!response.ok) {
    if (response.status === 401) {
      const message = token ? "管理员 Token 无效" : "管理员会话已失效，请重新登录";
      if (!$("setupForm").hidden) {
        resetSession();
        status(message, "error");
      }
      throw new Unauthorized(message);
    }
    if (response.status === 409 && path === "integrations") throw new Error("这个系统名称已有 Token");
    if (path.endsWith("/logs/latest")) {
      let code;
      try { code = (await response.json()).error?.code; } catch { /* No raw response text is shown. */ }
      const messages = { logs_upgrade_required: "请先在该节点安装最新升级包，再读取日志", logs_unavailable: "节点日志不可用，请检查本机日志目录权限", connector_disconnected: "节点未连接，暂时无法读取日志", connector_disabled: "节点已停用" };
      throw new Error(messages[code] || `节点日志读取失败（${response.status}）`);
    }
    if (binary && response.status === 503) throw new Error("所选系统的程序尚未发布，请选择其他系统");
    throw new Error(`中心请求失败（${response.status}），请检查节点状态后重试`);
  }
  if (response.status === 204) return;
  let result;
  if (binary) {
    const size = Number(response.headers.get("content-length"));
    if (size > 160 * 1024 * 1024) throw new Error("中心安装包超过大小限制");
    result = new Uint8Array(await response.arrayBuffer());
  } else {
    result = await response.json();
  }
  active.signal.throwIfAborted();
  return result;
}

function downloadFile(bytes, type, name) {
  const url = URL.createObjectURL(new Blob([bytes], { type }));
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  document.body.append(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

function renderScreenshotNodes(selected = "") {
  const select = $("screenshotNode");
  const choices = nodes.filter((node) => node.kind === "screenshot" && node.enabled);
  select.replaceChildren(...choices.map((node) =>
    new Option(`${node.name} · ${node.connected ? "已连接" : "未连接"}`, node.connectorId)));
  if (!choices.length) select.add(new Option("暂无可用的截图节点", ""));
  if (choices.some((node) => node.connectorId === selected)) select.value = selected;
  $("screenshotDownload").disabled = !select.value;
}

async function screenshotAction(button, action) {
  const active = session;
  if (!active || $("screenshotWorkspace").hidden || button.disabled) return;
  const message = $("screenshotStatus");
  button.disabled = true;
  message.textContent = "";
  try {
    await action(active);
  } catch (error) {
    if (!active.signal.aborted) message.textContent = error.message || "截图 Connector 操作失败";
  } finally {
    if (session === active) button.disabled = button.id === "screenshotDownload" && !$("screenshotNode").value;
  }
}

$("screenshotProvision").addEventListener("submit", (event) => {
  event.preventDefault();
  screenshotAction($("screenshotCreate"), async (active) => {
    const name = $("screenshotName").value.trim();
    if (!name || new TextEncoder().encode(name).length > 160) throw new Error("请填写简短的截图节点名称");
    const result = await request(active, setupPath, { method: "POST", body: { name, kind: "screenshot" } });
    nodes.push(result.connector);
    renderScreenshotNodes(result.connector.connectorId);
    $("screenshotProvision").reset();
    $("screenshotStatus").textContent = "截图节点已创建，可以下载部署包。";
  });
});
$("screenshotDownload").addEventListener("click", () => {
  screenshotAction($("screenshotDownload"), async (active) => {
    const id = encodeURIComponent($("screenshotNode").value);
    const bytes = await request(active, `${setupPath}/${id}/screenshot-installer`, { binary: true });
    downloadFile(bytes, "application/zip", `screenshot-connector-macos-${id.slice(0, 8)}.zip`);
    $("screenshotStatus").textContent = "部署包已下载，请在对应 Mac 上解压并配置。";
  });
});
$("screenshotRefresh").addEventListener("click", () => {
  screenshotAction($("screenshotRefresh"), async (active) => {
    const selected = $("screenshotNode").value;
    nodes = (await request(active, setupPath)).connectors;
    renderScreenshotNodes(selected);
    $("screenshotStatus").textContent = "连接状态已刷新。";
  });
});

function renderNodes(selected = "new") {
  const select = $("connector");
  select.replaceChildren(new Option("＋ 新建节点", "new"));
  for (const node of nodes) {
    if (node.kind === "screenshot") continue;
    const option = new Option(`${node.name} · ${node.enabled ? (node.connected ? "已连接" : "未连接") : "已停用"}`, node.connectorId);
    option.disabled = !node.enabled;
    select.add(option);
  }
  select.value = selected;
  updateFields();
  logs.setNodes(nodes);
}

function updateFields() {
  const isNew = $("connector").value === "new";
  const target = $("installTarget").value;
  const canUpgrade = !isNew && installationFiles(target).length > 0;
  if (!canUpgrade) $("upgrade").checked = false;
  const upgrade = $("upgrade").checked;
  $("upgradeRow").hidden = !canUpgrade;
  $("nameRow").hidden = !isNew;
  $("connectorName").disabled = !isNew;
  $("accountSection").hidden = upgrade;
  $("accountMergeHint").hidden = isNew || upgrade || !installationFiles(target).length;
  $("appleFields").disabled = upgrade || !$("appleEnabled").checked;
  $("googleFields").disabled = upgrade || !$("googleEnabled").checked;
  const system = { "windows-amd64": "Windows", "linux-amd64": "Linux", "darwin-arm64": "macOS" }[target];
  $("downloadButton").textContent = `生成并下载 ${system} ${upgrade ? "升级包" : "安装包"}`;
}

async function fileBytes(id, label) {
  const file = $(id).files[0];
  if (!file || !file.size || file.size > CREDENTIAL_LIMIT) throw new Error(`请选择有效的${label}（最大 256 KiB）`);
  return new Uint8Array(await file.arrayBuffer());
}

function resetSession() {
  if (session) {
    session.controller.abort();
    session = undefined;
  }
  nodes = [];
  automaticKeyId = "";
  $("adminCredential").value = "";
  $("setupForm").reset();
  renderNodes();
  $("setupForm").hidden = true;
  $("status").textContent = "";
  $("status").hidden = true;
  $("adminLock").hidden = true;
  $("loginButton").disabled = false;
  $("downloadButton").disabled = false;
  $("adminUnlock").hidden = !securePage;
  integrations.reset();
  logs.reset();
  forwarding.reset();
  $("screenshotWorkspace").open = false;
  $("screenshotWorkspace").hidden = true;
  $("screenshotStatus").textContent = "";
  $("screenshotProvision").reset();
  $("screenshotCreate").disabled = false;
  $("screenshotRefresh").disabled = false;
  renderScreenshotNodes();
}

async function restoreSession(token = "") {
  if (session) return;
  const signingIn = Boolean(token);
  const controller = new AbortController();
  const active = { controller, signal: controller.signal };
  session = active;
  $("loginButton").disabled = true;
  try {
    if (token) {
      status("正在验证管理员 Token…");
      await request(active, "session", { method: "POST", token });
      token = "";
    }
    const result = await request(active, setupPath);
    nodes = result.connectors;
    renderNodes();
    integrations.show();
    logs.show();
    renderScreenshotNodes();
    $("screenshotWorkspace").hidden = false;
    $("adminUnlock").hidden = true;
    $("adminLock").hidden = false;
    $("setupForm").hidden = false;
    forwarding.show();
    $("status").hidden = true;
  } catch (error) {
    if (!active.signal.aborted) {
      resetSession();
      if (signingIn || !(error instanceof Unauthorized)) status(error.message || "无法连接中心", "error");
    }
  }
}

async function lock() {
  resetSession();
  $("loginButton").disabled = true;
  try {
    await request({ signal: new AbortController().signal }, "session", { method: "DELETE" });
    $("loginButton").disabled = false;
    $("adminCredential").focus();
  } catch {
    $("adminLock").hidden = false;
    status("暂时无法结束会话，请重试「锁定管理」。", "error");
  }
}

// Local loopback is a secure context in Chrome, but credential entry still requires TLS.
const securePage = window.isSecureContext && window.location.protocol === "https:";
if (securePage) {
  $("adminUnlock").addEventListener("submit", (event) => {
    event.preventDefault();
    const token = $("adminCredential").value.trim();
    $("adminCredential").value = "";
    if (token) restoreSession(token);
    else status("请输入管理员 Token", "error");
  });
  $("adminLock").addEventListener("click", lock);
  window.addEventListener("pagehide", resetSession);
  window.addEventListener("pageshow", (event) => {
    if (event.persisted) restoreSession();
  });
  restoreSession();
} else {
  const https = new URL(window.location.href);
  https.protocol = "https:";
  $("adminHttpsLink").href = https;
  $("adminHttps").hidden = false;
}

for (const id of ["connector", "installTarget", "appleEnabled", "googleEnabled", "upgrade"]) $(id).addEventListener("change", updateFields);
$("appleFile").addEventListener("change", () => {
  const match = /^AuthKey_([A-Za-z0-9]+)\.p8$/i.exec($("appleFile").files[0]?.name || "");
  const current = $("keyId").value.trim();
  if (!current || current === automaticKeyId) $("keyId").value = match ? match[1] : "";
  automaticKeyId = match ? match[1] : "";
});

$("setupForm").addEventListener("submit", async (event) => {
  event.preventDefault();
  const active = session;
  if (!active || $("setupForm").hidden || $("downloadButton").disabled) return;
  $("downloadButton").disabled = true;
  try {
    const upgrade = $("upgrade").checked;
    status(upgrade ? "正在准备升级包…" : "正在读取并检查本机账号文件…");
    const account = {};
    if (!upgrade && $("appleEnabled").checked) account.apple = {
      issuerId: $("issuerId").value, keyId: $("keyId").value,
      content: await fileBytes("appleFile", " Apple .p8 文件"),
    };
    if (!upgrade && $("googleEnabled").checked) account.google = { content: await fileBytes("googleFile", " Google 服务账号 JSON") };
    if (!upgrade) validateAccount(account);
    const target = $("installTarget").value;
    const name = $("connectorName").value.trim();
    if ($("connector").value === "new" && (!name || new TextEncoder().encode(name).length > 160)) {
      throw new Error("请填写简短的节点名称");
    }
    // Obtain public launch assets before creating a registry entry.
    const launcher = target === "windows-amd64" ? "start.cmd" : "start.sh";
    const installFiles = installationFiles(target);
    const names = [launcher, ...installFiles];
    const [caResponse, ...scriptResponses] = await Promise.all(["ca.crt", ...names].map(
      (name) => fetch(new URL(name, window.location.href), {
        cache: "no-store", credentials: "omit", mode: "same-origin", redirect: "error", signal: active.signal,
      }),
    ));
    if (![caResponse, ...scriptResponses].every((response) => response.ok)) throw new Error("无法读取中心安装文件");
    const scriptTexts = await Promise.all(scriptResponses.map((response) => response.text()));
    const assets = { caBytes: new Uint8Array(await caResponse.arrayBuffer()), scripts: Object.fromEntries(names.map((name, index) => [name, scriptTexts[index]])) };
    let id = $("connector").value;
    if (id === "new") {
      status("正在创建节点身份…");
      const result = await request(active, setupPath, { method: "POST", body: { name } });
      id = result.connector.connectorId;
      nodes.push(result.connector);
      renderNodes(id); // Retain this identity on a failed download/retry.
    }
    status("正在获取节点程序…");
    const installer = await request(active, `${setupPath}/${encodeURIComponent(id)}/installer/${target}`, { binary: true });
    status(upgrade ? "正在生成仅含程序的升级包…" : "正在浏览器内合并配置与账号文件…");
    const result = await buildInstaller(installer, { target, account, upgrade }, assets, globalThis.JSZip);
    active.signal.throwIfAborted();
    downloadFile(result.bytes, "application/zip", result.fileName);
    const entry = installFiles[0] ?? launcher;
    const details = upgrade ? "升级会保留本机配置和商店凭据。" : installFiles.length ? "安装会合并本次提供的平台资料，保留其他平台，并设置后台和开机启动。" : "请保持启动进程运行。";
    status(`ZIP 已生成。完整解压后运行 ${entry}。${details}\n节点：${id}`, "success");
  } catch (error) {
    if (!active.signal.aborted) status(error.message || "生成失败，请检查信息后重试", "error");
  } finally {
    if (session === active) $("downloadButton").disabled = false;
  }
});
