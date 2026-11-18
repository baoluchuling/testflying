import { nodeLabel } from "./versions.mjs";

export function bindAccounts(request, getSession) {
  const $ = (id) => document.getElementById(id);
  let accounts = [], nodes = [], directories = new Map(), bindings = new Map(), selected = "", epoch = 0, busy = false;
  const mixed = "__mixed__";
  function groups() {
    const result = new Map(nodes.filter((node) => node.kind === "store").map((node) => [node.connectorId, []]));
    for (const account of accounts) {
      if (!result.has(account.storeConnectorId)) result.set(account.storeConnectorId, []);
      result.get(account.storeConnectorId).push(account);
    }
    return result;
  }
  const label = (id) => {
    const node = nodes.find((item) => item.connectorId === id);
    return node ? `${nodeLabel(node)} · ${id}` : id || "未配置";
  };
  function controls() {
    for (const control of $("accountsWorkspace").querySelectorAll("button, input, select")) control.disabled = busy;
    const picker = $("accountScreenshotNode"), save = $("accountScreenshotSave");
    if (save && picker) save.disabled = busy || picker.value === mixed || (picker.value !== "" && !nodes.some((node) => node.connectorId === picker.value && node.kind === "screenshot" && node.enabled));
  }
  function paragraph(parent, text) {
    const element = document.createElement("p"); element.className = "account-metadata"; element.textContent = text; parent.append(element);
  }
  function render() {
    $("accountGroups").replaceChildren();
    const inventory = groups();
    const selector = $("accountSelector"); selector.replaceChildren();
    for (const [id, sections] of inventory) {
      const option = document.createElement("option"); option.value = id;
      option.textContent = nodes.find((node) => node.connectorId === id)?.name || sections[0]?.name || id;
      selector.append(option);
    }
    selector.value = selected;
    if (inventory.has(selected)) {
      const nodeId = selected, sections = inventory.get(selected);
      const group = document.createElement("section"); group.className = "account"; group.dataset.storeConnectorId = nodeId;
      const title = document.createElement("h3"); title.textContent = nodes.find((node) => node.connectorId === nodeId)?.name || sections[0]?.name || nodeId; group.append(title);
      paragraph(group, `商店节点：${label(nodeId)}`);
      const bindingLabel = document.createElement("label"); bindingLabel.htmlFor = "accountScreenshotNode"; bindingLabel.textContent = "账号截图节点（iOS / Android 共用）";
      const picker = document.createElement("select"); picker.id = "accountScreenshotNode";
      const addOption = (value, text, disabled = false) => {
        const option = document.createElement("option"); option.value = value; option.textContent = text; option.disabled = disabled; picker.append(option);
      };
      const previous = new Set(sections.map((account) => account.screenshotConnectorId ?? null));
      const conflict = previous.size > 1 || sections.some((account) => account.migrationError?.startsWith("legacy_screenshot_"));
      const value = bindings.has(nodeId) ? bindings.get(nodeId) : conflict ? mixed : [...previous][0] ?? "";
      if (value === mixed) addOption(mixed, "历史配置不一致，请选择统一的截图节点", true);
      addOption("", "不配置截图节点");
      for (const node of nodes.filter((item) => item.kind === "screenshot" && item.enabled)) addOption(node.connectorId, label(node.connectorId));
      if (value && value !== mixed && !nodes.some((node) => node.connectorId === value && node.kind === "screenshot" && node.enabled)) addOption(value, `${label(value)} · 当前不可用`, true);
      picker.value = value || "";
      const save = document.createElement("button"); save.id = "accountScreenshotSave"; save.type = "button"; save.textContent = "保存截图节点";
      picker.addEventListener("change", controls);
      save.addEventListener("click", () => {
        const connectorId = picker.value || null;
        if (connectorId === mixed) return;
        perform(async (active, current) => {
          await request(active, `accounts/store-connectors/${encodeURIComponent(nodeId)}/screenshot-connector`, { method: "PUT", body: { connectorId } });
          await load(active, current);
          return "账号截图节点已保存，iOS 和 Android 共用此配置。已创建任务不受影响。";
        });
      });
      group.append(bindingLabel, picker, save);
      if (!sections.length) paragraph(group, "尚无平台目录。商店节点上报后会自动显示应用，并沿用此截图节点配置。");
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
    $("accountAppsEmpty").hidden = inventory.size > 0;
    controls();
  }
  async function load(active, current) {
    const [inventory, result] = await Promise.all([request(active, "connectors"), request(active, "accounts")]);
    if (current !== epoch || active.signal.aborted) return;
    nodes = inventory.connectors; accounts = result.accounts;
    bindings = new Map((result.screenshotBindings || []).map((binding) => [binding.storeConnectorId, binding.connectorId]));
    const grouped = groups();
    if (!grouped.has(selected)) selected = grouped.keys().next().value || "";
    directories = new Map(); render();
    const selectedAccounts = grouped.get(selected) || [], next = new Map();
    // Read center caches only, with bounded concurrency; no provider refreshes.
    for (let index = 0; index < selectedAccounts.length; index += 4) {
      await Promise.all(selectedAccounts.slice(index, index + 4).map(async (account) => {
        try { next.set(account.id, (await request(active, `accounts/${encodeURIComponent(account.id)}/apps`)).apps); }
        catch (error) { if (active.signal.aborted) throw error; }
      }));
      if (current !== epoch || active.signal.aborted) return;
    }
    if (current !== epoch || active.signal.aborted) return;
    directories = next;
    const identifiers = new Set([...next.values()].flat().map((app) => `${app.platform}/${app.bundleIdentifier}`));
    $("accountMigrationIssues").textContent = (result.migrationIssues || []).filter((issue) => identifiers.has(`${issue.platform}/${issue.bundleIdentifier}`)).map((issue) => `${issue.platform} / ${issue.bundleIdentifier}：${issue.code}`).join("\n");
    render();
    if (next.size !== selectedAccounts.length) throw new Error("部分目录读取失败，请重新读取。");
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
  $("accountSelector").addEventListener("change", () => {
    selected = $("accountSelector").value; directories = new Map();
    $("accountMigrationIssues").textContent = ""; render(); perform(load);
  });
  return {
    show() { $("accountsWorkspace").hidden = false; $("accountsWorkspace").open = true; perform(load); },
    reset() {
      epoch++; busy = false; accounts = []; nodes = []; directories = new Map(); bindings = new Map(); selected = "";
      $("accountsWorkspace").hidden = true; $("accountsWorkspace").open = false;
      $("accountMigrationIssues").textContent = ""; $("accountsStatus").textContent = ""; render();
    },
  };
}
