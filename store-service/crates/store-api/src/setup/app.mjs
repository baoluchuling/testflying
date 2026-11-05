import { CREDENTIAL_LIMIT, validateAccount, buildInstaller, installationFiles } from "./packager.mjs";

const $ = (id) => document.getElementById(id);
const apiBase = new URL("./api/", window.location.href);
let busy = false;
let nodes = [];
let automaticKeyId = "";

function status(message, kind = "info") {
  $("status").hidden = false;
  $("status").textContent = message;
  $("status").dataset.kind = kind;
}

async function request(path, { method = "GET", body, binary = false } = {}) {
  const response = await fetch(new URL(path, apiBase), {
    method,
    cache: "no-store",
    credentials: "omit",
    referrerPolicy: "no-referrer",
    headers: body ? { "Content-Type": "application/json" } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  });
  if (!response.ok) {
    if (binary && response.status === 503) throw new Error("所选系统的程序尚未发布，请选择其他系统");
    throw new Error(`中心请求失败（${response.status}），请检查节点状态后重试`);
  }
  if (binary) {
    const size = Number(response.headers.get("content-length"));
    if (size > 160 * 1024 * 1024) throw new Error("中心安装包超过大小限制");
    return new Uint8Array(await response.arrayBuffer());
  }
  return response.json();
}

function renderNodes(selected = "new") {
  const select = $("connector");
  select.replaceChildren(new Option("＋ 新建节点", "new"));
  for (const node of nodes) {
    const option = new Option(`${node.name} · ${node.enabled ? (node.connected ? "已连接" : "未连接") : "已停用"}`, node.connectorId);
    option.disabled = !node.enabled;
    select.add(option);
  }
  select.value = selected;
  updateFields();
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

async function loadNodes() {
  $("retryConnection").hidden = true;
  try {
    status("正在连接中心…");
    const result = await request("connectors");
    nodes = result.connectors;
    renderNodes();
    $("setupForm").hidden = false;
    $("status").hidden = true;
  } catch (error) {
    status(error.message || "无法连接中心", "error");
    $("retryConnection").hidden = false;
  }
}

$("retryConnection").addEventListener("click", loadNodes);

for (const id of ["connector", "installTarget", "appleEnabled", "googleEnabled", "upgrade"]) $(id).addEventListener("change", updateFields);
$("appleFile").addEventListener("change", () => {
  const match = /^AuthKey_([A-Za-z0-9]+)\.p8$/i.exec($("appleFile").files[0]?.name || "");
  const current = $("keyId").value.trim();
  if (!current || current === automaticKeyId) $("keyId").value = match ? match[1] : "";
  automaticKeyId = match ? match[1] : "";
});

$("setupForm").addEventListener("submit", async (event) => {
  event.preventDefault();
  if (busy) return;
  busy = true;
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
      (name) => fetch(new URL(name, window.location.href), { cache: "no-store" }),
    ));
    if (![caResponse, ...scriptResponses].every((response) => response.ok)) throw new Error("无法读取中心安装文件");
    const scriptTexts = await Promise.all(scriptResponses.map((response) => response.text()));
    const assets = { caBytes: new Uint8Array(await caResponse.arrayBuffer()), scripts: Object.fromEntries(names.map((name, index) => [name, scriptTexts[index]])) };
    let id = $("connector").value;
    if (id === "new") {
      status("正在创建节点身份…");
      const result = await request("connectors", { method: "POST", body: { name } });
      id = result.connector.connectorId;
      nodes.push(result.connector);
      renderNodes(id); // Retain this identity on a failed download/retry.
    }
    status("正在获取节点程序…");
    const installer = await request(`connectors/${encodeURIComponent(id)}/installer/${target}`, { binary: true });
    status(upgrade ? "正在生成仅含程序的升级包…" : "正在浏览器内合并配置与账号文件…");
    const result = await buildInstaller(installer, { target, account, upgrade }, assets, globalThis.JSZip);
    const url = URL.createObjectURL(new Blob([result.bytes], { type: "application/zip" }));
    const link = document.createElement("a");
    link.href = url;
    link.download = result.fileName;
    document.body.append(link);
    link.click();
    link.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
    const entry = installFiles[0] ?? launcher;
    const details = upgrade ? "升级会保留本机配置和商店凭据。" : installFiles.length ? "安装会立即启动并设置开机启动。" : "请保持启动进程运行。";
    status(`ZIP 已生成。完整解压后运行 ${entry}。${details}\n节点：${id}`, "success");
  } catch (error) {
    status(error.message || "生成失败，请检查信息后重试", "error");
  } finally {
    busy = false;
    $("downloadButton").disabled = false;
  }
});

loadNodes();
