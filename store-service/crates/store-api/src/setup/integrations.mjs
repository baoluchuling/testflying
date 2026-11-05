const $ = (id) => document.getElementById(id);
const center = new URL("../", window.location.href);
let adminToken = "";
let nodes = [];
let busy = false;

function message(text) {
  $("integrationStatus").textContent = text;
}

function showToken(token = "") {
  $("integrationToken").value = token;
  $("integrationSecret").hidden = !token;
}

async function request(path, { method = "GET", body } = {}) {
  const response = await fetch(new URL(path, center), {
    method,
    cache: "no-store",
    credentials: "omit",
    headers: { Authorization: `Bearer ${adminToken}`, ...(body ? { "Content-Type": "application/json" } : {}) },
    body: body ? JSON.stringify(body) : undefined,
  });
  if (response.status === 401) throw new Error("管理员 Token 无效");
  if (response.status === 409) throw new Error("这个系统名称已有 Token");
  if (!response.ok) throw new Error(`凭据请求失败（${response.status}）`);
  return response.status === 204 ? undefined : response.json();
}

async function run(action) {
  if (busy) return;
  busy = true;
  try {
    await action();
  } catch (error) {
    message(error.message || "凭据操作失败");
  } finally {
    busy = false;
  }
}

function actionButton(label, action) {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "quiet";
  button.textContent = label;
  button.addEventListener("click", () => run(action));
  return button;
}

async function refreshList() {
  const result = await request("v1/integrations");
  const list = $("integrationList");
  list.replaceChildren();
  for (const integration of result.integrations) {
    const row = document.createElement("li");
    const names = integration.connectorIds.map((id) => nodes.find((node) => node.connectorId === id)?.name ?? id);
    row.append(document.createTextNode(`${integration.name} · ${names.join("、")} `));
    const path = `v1/integrations/${encodeURIComponent(integration.id)}`;
    row.append(actionButton("取回 Token", async () => {
      showToken((await request(path + "/token")).token);
      message("可以复制这枚接入 Token。");
    }));
    row.append(actionButton("撤销", async () => {
      if (!window.confirm(`撤销「${integration.name}」的 Token？`)) return;
      await request(path, { method: "DELETE" });
      showToken();
      await refreshList();
      message("Token 已撤销。");
    }));
    list.append(row);
  }
}

function lock() {
  adminToken = "";
  nodes = [];
  showToken();
  $("integrationAdmin").value = "";
  $("integrationIssue").reset();
  $("integrationList").replaceChildren();
  $("integrationNodes").replaceChildren();
  $("integrationPanel").hidden = true;
  $("integrationUnlock").hidden = false;
  message("");
}

if (!window.isSecureContext) {
  const https = new URL(window.location.href);
  https.protocol = "https:";
  $("integrationHttpsLink").href = https;
  $("integrationHttps").hidden = false;
} else {
  $("integrationUnlock").hidden = false;
  $("integrationUnlock").addEventListener("submit", (event) => {
    event.preventDefault();
    run(async () => {
      adminToken = $("integrationAdmin").value.trim();
      $("integrationAdmin").value = "";
      try {
        const response = await fetch(new URL("./api/connectors", window.location.href), { cache: "no-store" });
        if (!response.ok) throw new Error("无法读取节点列表");
        nodes = (await response.json()).connectors;
        await refreshList();
        $("integrationNodes").replaceChildren(...nodes.filter((node) => node.enabled)
          .map((node) => new Option(node.name, node.connectorId)));
        $("integrationUnlock").hidden = true;
        $("integrationPanel").hidden = false;
        message("");
      } catch (error) {
        lock();
        throw error;
      }
    });
  });
  $("integrationIssue").addEventListener("submit", (event) => {
    event.preventDefault();
    run(async () => {
      const connectorIds = Array.from($("integrationNodes").selectedOptions, (option) => option.value);
      const result = await request("v1/integrations", {
        method: "POST",
        body: { name: $("integrationName").value.trim(), connectorIds },
      });
      showToken(result.token);
      $("integrationIssue").reset();
      await refreshList();
      message("Token 已发放，可复制到该系统的后端。");
    });
  });
  $("integrationLock").addEventListener("click", () => run(lock));
  $("integrationCopy").addEventListener("click", () => run(async () => {
    const field = $("integrationToken");
    try {
      await navigator.clipboard.writeText(field.value);
      message("Token 已复制。");
    } catch {
      field.focus();
      field.select();
      message("已选中 Token，请使用系统快捷键复制。");
    }
  }));
  window.addEventListener("pagehide", lock);
}
