export function bindAccounts(request, getSession) {
  const $ = (id) => document.getElementById(id);
  let accounts = [], nodes = [], directories = new Map(), epoch = 0, busy = false;
  const label = (id) => {
    const node = nodes.find((item) => item.connectorId === id);
    return node ? `${node.name} · ${id} · ${node.enabled ? (node.connected ? "已连接" : "未连接") : "已停用"}` : id || "未配置";
  };
  function controls() {
    for (const control of $("accountsWorkspace").querySelectorAll("button, input")) control.disabled = busy;
  }
  function paragraph(parent, text) {
    const element = document.createElement("p"); element.className = "account-metadata"; element.textContent = text; parent.append(element);
  }
  function render() {
    $("accountGroups").replaceChildren();
    const groups = new Map();
    for (const account of accounts) {
      if (!groups.has(account.storeConnectorId)) groups.set(account.storeConnectorId, []);
      groups.get(account.storeConnectorId).push(account);
    }
    for (const [nodeId, sections] of groups) {
      const group = document.createElement("section"); group.className = "account"; group.dataset.storeConnectorId = nodeId;
      const title = document.createElement("h3"); title.textContent = nodes.find((node) => node.connectorId === nodeId)?.name || sections[0].name; group.append(title);
      paragraph(group, `商店节点：${label(nodeId)}`);
      for (const account of sections.sort((a, b) => b.platform.localeCompare(a.platform))) {
        const section = document.createElement("section"); section.dataset.accountId = account.id;
        const heading = document.createElement("h4"); heading.textContent = account.platform === "ios" ? "iOS 目录" : "Android 目录"; section.append(heading);
        paragraph(section, `目录 ID：${account.id}\n状态：${account.catalogStatus} · 错误：${account.catalogError || "无"}\n最后成功：${account.lastCatalogAt == null ? "尚无" : new Date(account.lastCatalogAt * 1000).toLocaleString()}\n账号：${account.enabled ? "启用" : "停用"} · 迁移问题：${account.migrationError || "无"}\n截图节点：${label(account.screenshotConnectorId)}`);
        const apps = directories.get(account.id);
        if (!apps) paragraph(section, "目录读取失败，请重新读取。");
        else if (!apps.length) paragraph(section, "商店尚未返回支持的应用。");
        const list = document.createElement("ul");
        for (const app of apps || []) {
          const row = document.createElement("li"); row.className = "account-app"; row.dataset.identifier = app.bundleIdentifier;
          paragraph(row, `${app.name} · ${app.platform} / ${app.bundleIdentifier}\n商店应用 ID：${app.storeAppId || "无"} · 路由：${app.resolutionStatus}\n截图路由：${app.screenshotResolutionStatus}`);
          const toggle = document.createElement("label"); toggle.className = "toggle";
          const input = document.createElement("input"); input.type = "checkbox"; input.checked = app.enabled; input.setAttribute("aria-label", `${app.platform} / ${app.bundleIdentifier} 应用转发`);
          const text = document.createElement("span"); text.textContent = "开启应用转发"; toggle.append(input, text); row.append(toggle);
          input.addEventListener("change", () => {
            const enabled = input.checked;
            perform(async (active, current) => {
              await request(active, `accounts/${encodeURIComponent(account.id)}/apps/${encodeURIComponent(app.platform)}/${encodeURIComponent(app.bundleIdentifier)}/forwarding`, { method: "PUT", body: { enabled } });
              await load(active, current);
              return enabled ? "应用已开启转发。" : "应用已关闭转发，已创建任务继续执行。";
            });
          });
          list.append(row);
        }
        section.append(list); group.append(section);
      }
      $("accountGroups").append(group);
    }
    $("accountAppsEmpty").hidden = accounts.length > 0;
    controls();
  }
  async function load(active, current) {
    const [inventory, result] = await Promise.all([request(active, "connectors"), request(active, "accounts")]);
    const next = new Map();
    // Read center caches only, with bounded concurrency; no provider refreshes.
    for (let index = 0; index < result.accounts.length; index += 4) {
      await Promise.all(result.accounts.slice(index, index + 4).map(async (account) => {
        try { next.set(account.id, (await request(active, `accounts/${encodeURIComponent(account.id)}/apps`)).apps); }
        catch (error) { if (active.signal.aborted) throw error; }
      }));
      if (current !== epoch || active.signal.aborted) return;
    }
    if (current !== epoch || active.signal.aborted) return;
    nodes = inventory.connectors; accounts = result.accounts; directories = next;
    $("accountMigrationIssues").textContent = (result.migrationIssues || []).map((issue) => `${issue.platform} / ${issue.bundleIdentifier}：${issue.code}`).join("\n");
    render();
    if (next.size !== accounts.length) throw new Error("部分目录读取失败，请重新读取。");
  }
  async function perform(action) {
    const active = getSession(); if (!active || busy) return;
    const current = ++epoch; busy = true; controls();
    $("accountsStatus").textContent = "正在读取目录…";
    try {
      const message = await action(active, current);
      if (current === epoch && !active.signal.aborted) $("accountsStatus").textContent = message || "账号目录已读取。";
    } catch (error) {
      if (current === epoch && !active.signal.aborted) $("accountsStatus").textContent = error.message || "目录操作失败，请重试。";
    } finally { if (current === epoch) { busy = false; render(); } }
  }
  $("accountsReload").addEventListener("click", () => perform(load));
  return {
    show() { $("accountsWorkspace").hidden = false; $("accountsWorkspace").open = true; perform(load); },
    reset() {
      epoch++; busy = false; accounts = []; nodes = []; directories = new Map();
      $("accountsWorkspace").hidden = true; $("accountsWorkspace").open = false;
      $("accountMigrationIssues").textContent = ""; $("accountsStatus").textContent = ""; render();
    },
  };
}
