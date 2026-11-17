export function bindAccounts(request, getSession) {
  const $ = (id) => document.getElementById(id);
  let accounts = [], nodes = [], apps = [], epoch = 0, busy = false;
  const selected = () => accounts.find((account) => account.id === $("managedAccount").value);
  const bindable = new Set(["resolved", "bound_elsewhere", "account_ambiguous"]);
  function nodeLabel(id) {
    const node = nodes.find((item) => item.connectorId === id);
    return node ? `${node.name} · ${id} · ${node.enabled ? (node.connected ? "已连接" : "未连接") : "已停用"}` : id || "未配置";
  }
  function controls() {
    for (const button of $("accountsWorkspace").querySelectorAll("button")) button.disabled = busy;
    $("managedAccount").disabled = busy;
    $("managedAccountPlatform").disabled = busy || Boolean(selected());
    $("managedAccountEnabled").disabled = busy || !selected();
    $("accountCatalogRefresh").disabled = busy || !selected();
    $("accountAcceptIdentity").hidden = selected()?.catalogStatus !== "identity_changed";
  }
  function fields() {
    const value = selected();
    $("managedAccountName").value = value?.name || "";
    $("managedAccountPlatform").value = value?.platform || "ios";
    $("managedAccountEnabled").checked = value?.enabled ?? true;
    for (const [id, kind, current] of [["accountStoreNode", "store", value?.storeConnectorId], ["accountScreenshotNode", "screenshot", value?.screenshotConnectorId]]) {
      const select = $(id);
      select.replaceChildren(new Option(kind === "store" ? "请选择商店节点" : "不配置截图节点", ""));
      for (const node of nodes.filter((item) => (item.kind || "store") === kind)) select.add(new Option(nodeLabel(node.connectorId), node.connectorId));
      if (current && !nodes.some((node) => node.connectorId === current)) select.add(new Option(`节点不可用 · ${current}`, current));
      select.value = current || "";
    }
    $("accountSave").textContent = value ? "保存账号" : "创建账号";
    $("accountMetadata").textContent = value ? `账号 ID：${value.id}\n目录：${value.catalogStatus} · 错误：${value.catalogError || "无"}\n最后成功目录：${value.lastCatalogAt == null ? "尚无" : new Date(value.lastCatalogAt * 1000).toLocaleString()}\n迁移问题：${value.migrationError || "无"}\n商店节点：${nodeLabel(value.storeConnectorId)}\n截图节点：${nodeLabel(value.screenshotConnectorId)}` : "账号平台创建后不可更改。一个截图节点可供多个账号使用。";
    controls();
  }
  function renderApps() {
    $("accountApps").replaceChildren();
    for (const app of apps) {
      const row = document.createElement("li"); row.className = "account-app";
      const text = document.createElement("p");
      text.textContent = `${app.name} · ${app.platform} / ${app.bundleIdentifier}\n商店应用 ID：${app.storeAppId || "无"} · 账号归属：${app.resolutionStatus}\n截图路由：${app.screenshotEnabled ? "可用" : "不可用"} · ${app.screenshotResolutionStatus}`;
      row.append(text);
      if (bindable.has(app.resolutionStatus)) {
        const button = document.createElement("button"); button.type = "button"; button.className = "quiet";
        button.textContent = "绑定到此账号"; button.disabled = busy;
        button.addEventListener("click", () => perform(async (active) => {
          const account = selected(); if (!account) return;
          await request(active, `apps/${encodeURIComponent(app.platform)}/${encodeURIComponent(app.bundleIdentifier)}/account`, { method: "PUT", body: { accountId: account.id } });
          await loadApps(active); return "应用已绑定到此账号。";
        }));
        row.append(button);
      }
      $("accountApps").append(row);
    }
    $("accountAppsEmpty").hidden = apps.length > 0 || !selected();
  }
  async function loadApps(active) {
    const value = selected();
    apps = value ? (await request(active, `accounts/${encodeURIComponent(value.id)}/apps`)).apps : [];
    renderApps();
  }
  async function load(active, preferred = $("managedAccount").value) {
    const [inventory, result] = await Promise.all([request(active, "connectors"), request(active, "accounts")]);
    nodes = inventory.connectors; accounts = result.accounts;
    $("managedAccount").replaceChildren(new Option("＋ 新建账号", ""), ...accounts.map((value) => new Option(`${value.name} · ${value.platform} · ${value.enabled ? "启用" : "停用"}`, value.id)));
    $("managedAccount").value = accounts.some((value) => value.id === preferred) ? preferred : "";
    $("accountMigrationIssues").textContent = (result.migrationIssues || []).map((issue) => `${issue.platform} / ${issue.bundleIdentifier}：${issue.code}`).join("\n");
    apps = []; fields(); renderApps(); await loadApps(active);
  }
  async function perform(action) {
    const active = getSession(); if (!active || busy) return;
    const current = ++epoch; busy = true; controls();
    $("accountsStatus").textContent = "正在处理账号配置…";
    try {
      const message = await action(active);
      if (current === epoch && !active.signal.aborted) $("accountsStatus").textContent = message || "账号目录已读取。";
    } catch (error) {
      if (current === epoch && !active.signal.aborted) $("accountsStatus").textContent = error.message || "账号操作失败，请重试。";
    } finally { if (current === epoch) { busy = false; controls(); } }
  }
  $("managedAccount").addEventListener("change", () => perform(async (active) => { apps = []; renderApps(); fields(); await loadApps(active); }));
  $("accountsReload").addEventListener("click", () => perform((active) => load(active)));
  $("managedAccountForm").addEventListener("submit", (event) => {
    event.preventDefault(); perform(async (active) => {
      const name = $("managedAccountName").value.trim();
      if (!name || new TextEncoder().encode(name).length > 160) throw new Error("请填写简短的账号名称");
      const value = selected(), storeConnectorId = $("accountStoreNode").value;
      if (!storeConnectorId) throw new Error("请选择商店节点");
      const screenshotConnectorId = $("accountScreenshotNode").value || null;
      const body = { name };
      if (value) {
        body.enabled = $("managedAccountEnabled").checked;
        if (storeConnectorId !== value.storeConnectorId) body.storeConnectorId = storeConnectorId;
        if (screenshotConnectorId !== value.screenshotConnectorId) body.screenshotConnectorId = screenshotConnectorId;
      } else {
        body.platform = $("managedAccountPlatform").value;
        body.storeConnectorId = storeConnectorId;
        if (screenshotConnectorId !== null) body.screenshotConnectorId = screenshotConnectorId;
      }
      const result = await request(active, value ? `accounts/${encodeURIComponent(value.id)}` : "accounts", { method: value ? "PATCH" : "POST", body });
      await load(active, result.account.id); return "账号已保存。目录更新由商店节点上报，刷新后可查看应用。";
    });
  });
  $("accountCatalogRefresh").addEventListener("click", () => perform(async (active) => {
    await request(active, `accounts/${encodeURIComponent(selected().id)}/refresh`, { method: "POST", body: {} });
    return "已请求刷新目录；节点完成后请点击「重新读取账号与目录」。";
  }));
  $("accountAcceptIdentity").addEventListener("click", () => perform(async (active) => {
    const id = selected().id;
    await request(active, `accounts/${encodeURIComponent(id)}`, { method: "PATCH", body: { acceptIdentityChange: true } });
    await load(active, id); return "已接受新身份，旧目录不再用于绑定。请刷新目录。";
  }));
  return {
    show() { $("accountsWorkspace").hidden = false; perform((active) => load(active)); },
    reset() {
      epoch++; busy = false; accounts = []; nodes = []; apps = [];
      $("accountsWorkspace").hidden = true; $("accountsWorkspace").open = false;
      $("managedAccount").replaceChildren(); $("managedAccountForm").reset();
      $("accountMetadata").textContent = ""; $("accountMigrationIssues").textContent = ""; $("accountsStatus").textContent = "";
      renderApps(); controls();
    },
  };
}
