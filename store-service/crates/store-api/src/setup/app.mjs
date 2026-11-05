import { CREDENTIAL_LIMIT, validateAccount, buildInstaller } from "./packager.mjs";

const $ = (id) => document.getElementById(id);
const apiBase = new URL("../", window.location.href);
let token = "";
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
    headers: { Authorization: `Bearer ${token}`, ...(body ? { "Content-Type": "application/json" } : {}) },
    body: body ? JSON.stringify(body) : undefined,
  });
  if (!response.ok) {
    if (response.status === 401) throw new Error("管理员 Token 无效或已更换，请退出后重新登录");
    if (response.status === 503) throw new Error("所选系统的程序尚未发布，请选择其他系统");
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
  $("nameRow").hidden = !isNew;
  $("connectorName").disabled = !isNew;
  $("appleFields").disabled = !$("appleEnabled").checked;
  $("googleFields").disabled = !$("googleEnabled").checked;
  const system = { "windows-amd64": "Windows", "linux-amd64": "Linux", "darwin-arm64": "macOS" }[$("installTarget").value];
  $("downloadButton").textContent = `生成并下载 ${system} 安装包`;
}

async function fileBytes(id, label) {
  const file = $(id).files[0];
  if (!file || !file.size || file.size > CREDENTIAL_LIMIT) throw new Error(`请选择有效的${label}（最大 256 KiB）`);
  return new Uint8Array(await file.arrayBuffer());
}

$("loginForm").addEventListener("submit", async (event) => {
  event.preventDefault();
  if (busy) return;
  busy = true;
  $("loginButton").disabled = true;
  token = $("adminToken").value.trim();
  try {
    status("正在连接中心…");
    const result = await request("v1/connectors");
    nodes = result.connectors;
    renderNodes();
    $("adminToken").value = "";
    $("loginPanel").hidden = true;
    $("setupForm").hidden = false;
    $("logout").hidden = false;
    status("已连接中心。选择文件后即可生成安装包。");
  } catch (error) {
    token = "";
    status(error.message || "无法连接中心", "error");
  } finally {
    busy = false;
    $("loginButton").disabled = false;
  }
});

$("logout").addEventListener("click", () => {
  if (busy) return;
  token = "";
  nodes = [];
  automaticKeyId = "";
  $("setupForm").reset();
  $("setupForm").hidden = true;
  $("loginPanel").hidden = false;
  $("logout").hidden = true;
  $("status").hidden = true;
  renderNodes();
  $("adminToken").focus();
});

for (const id of ["connector", "installTarget", "appleEnabled", "googleEnabled"]) $(id).addEventListener("change", updateFields);
$("appleFile").addEventListener("change", () => {
  const match = /^AuthKey_([A-Za-z0-9]+)\.p8$/i.exec($("appleFile").files[0]?.name || "");
  const current = $("keyId").value.trim();
  if (!current || current === automaticKeyId) $("keyId").value = match ? match[1] : "";
  automaticKeyId = match ? match[1] : "";
});

$("setupForm").addEventListener("submit", async (event) => {
  event.preventDefault();
  if (busy || !token) return;
  busy = true;
  $("downloadButton").disabled = true;
  $("logout").disabled = true;
  try {
    status("正在读取并检查本机账号文件…");
    const account = {};
    if ($("appleEnabled").checked) account.apple = {
      issuerId: $("issuerId").value, keyId: $("keyId").value,
      content: await fileBytes("appleFile", " Apple .p8 文件"),
    };
    if ($("googleEnabled").checked) account.google = { content: await fileBytes("googleFile", " Google 服务账号 JSON") };
    validateAccount(account);
    const target = $("installTarget").value;
    const name = $("connectorName").value.trim();
    if ($("connector").value === "new" && (!name || new TextEncoder().encode(name).length > 160)) {
      throw new Error("请填写简短的节点名称");
    }
    // Obtain public launch assets before creating a registry entry.
    const [caResponse, cmdResponse, shResponse] = await Promise.all([
      fetch(new URL("ca.crt", window.location.href), { cache: "no-store" }),
      fetch(new URL("start.cmd", window.location.href), { cache: "no-store" }),
      fetch(new URL("start.sh", window.location.href), { cache: "no-store" }),
    ]);
    if (![caResponse, cmdResponse, shResponse].every((response) => response.ok)) throw new Error("无法读取中心启动文件");
    const assets = { caBytes: new Uint8Array(await caResponse.arrayBuffer()), windowsLauncher: await cmdResponse.text(), unixLauncher: await shResponse.text() };
    let id = $("connector").value;
    if (id === "new") {
      status("正在创建节点身份…");
      const result = await request("v1/connectors", { method: "POST", body: { name } });
      id = result.connector.connectorId;
      nodes.push(result.connector);
      renderNodes(id); // Retain this identity on a failed download/retry.
    }
    status("正在获取节点程序…");
    const installer = await request(`v1/connectors/${encodeURIComponent(id)}/installer/${target}`, { binary: true });
    status("正在浏览器内合并配置与账号文件…");
    const result = await buildInstaller(installer, { target, account }, assets, globalThis.JSZip);
    const url = URL.createObjectURL(new Blob([result.bytes], { type: "application/zip" }));
    const link = document.createElement("a");
    link.href = url;
    link.download = result.fileName;
    document.body.append(link);
    link.click();
    link.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
    const startup = target === "windows-amd64" ? "start.cmd" : "start.sh";
    status(`安装包已生成。解压整个 ZIP 后运行 ${startup}，无需修改路径或节点配置。\n节点：${id}\n应用绑定仍由中心维护，本次下载不会修改应用映射。`, "success");
  } catch (error) {
    status(error.message || "生成失败，请检查信息后重试", "error");
  } finally {
    busy = false;
    $("downloadButton").disabled = false;
    $("logout").disabled = false;
  }
});

updateFields();
