const STATES = {
  notified: "已通知", downloading: "下载中", draining: "等待任务结束", installing: "升级中",
  awaiting_confirmation: "待中心确认", succeeded: "升级成功", rolled_back: "已回滚",
  failed: "升级失败", skipped: "已跳过", expired: "批次已过期", cleanup_pending: "清理待完成",
};
const REASONS = {
  manual_install_required: "需手动安装一次新版安装包以启用后台升级",
  unsupported_installation: "当前安装方式不支持后台升级，请手动安装",
  current: "已是最新版本", newer: "当前版本高于发布版本", unknown_version: "版本未知，请手动安装一次新版",
  release_unavailable: "对应系统的升级包尚不可用",
};

function compareVersions(left, right) {
  const pattern = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
  if (!pattern.test(left || "") || !pattern.test(right || "")) return undefined;
  const a = left.split(".").map(BigInt), b = right.split(".").map(BigInt);
  for (let i = 0; i < 3; i++) if (a[i] !== b[i]) return a[i] < b[i] ? -1 : 1;
  return 0;
}

export function bindStoreUpgrades(request, getSession) {
  const $ = (id) => document.getElementById(id);
  let overview, batch, busy = false, epoch = 0, timer;
  const storeNodes = () => (overview?.nodes || []).filter((node) => node.kind === "store");
  const onlineNodes = () => storeNodes().filter((node) => node.online && node.enabled);
  const isActive = () => batch?.state === "active";
  function cancelTimer() { clearTimeout(timer); timer = undefined; }
  function controls() {
    $("storeUpgradeStart").textContent = `升级全部在线商店节点（${onlineNodes().length}）`;
    const hasNewVersion = onlineNodes().some((node) => compareVersions(node.version, overview?.latestVersion) === -1);
    $("storeUpgradeStart").disabled = busy || isActive() || !hasNewVersion;
    $("storeUpgradeRefresh").disabled = busy;
  }
  function render() {
    $("storeUpgradeVersion").textContent = `最新商店版本：${overview?.latestVersion || "尚未发布"}`;
    $("storeUpgradeBatch").textContent = batch ? `批次：${batch.batchId} · 目标 ${batch.version} · ${isActive() ? "进行中" : "已结束"}` : "";
    const results = new Map((batch?.nodes || []).map((node) => [node.connectorId, node]));
    const rows = storeNodes().map((node) => ({ node, result: results.get(node.connectorId) }));
    // Retain the original batch snapshot even if a node has since been removed or disabled.
    for (const result of batch?.nodes || []) {
      if (overview?.nodes.some((node) => node.connectorId === result.connectorId && node.kind !== "store")) continue;
      if (!rows.some(({ node }) => node.connectorId === result.connectorId)) rows.push({ node: result, result });
    }
    $("storeUpgradeNodes").replaceChildren(...rows.map(({ node, result }) => {
      const row = document.createElement("li");
      row.dataset.connectorId = node.connectorId;
      const label = document.createElement("strong"); label.textContent = node.name || node.connectorId;
      const metadata = document.createElement("p");
      metadata.textContent = `当前版本 ${node.version || "未知"} · ${node.target || "系统未知"} · ${node.online === undefined ? "批次记录" : node.online ? "在线" : "未连接"}`;
      const state = document.createElement("p");
      const reason = result?.reason || (!node.supported ? node.reason : undefined);
      const comparison = compareVersions(node.version, overview?.latestVersion);
      const summary = result ? STATES[result.state] || "状态待确认"
        : !node.enabled ? "节点已停用" : !node.online ? "本次跳过，未连接节点不会自动加入批次"
        : reason ? REASONS[reason] || "当前节点暂不支持后台升级"
        : comparison === 0 ? "已是最新版本，将跳过" : comparison === 1 ? "高于发布版本，将跳过"
        : comparison === -1 ? "有新版可用" : "版本未知，请手动安装一次新版";
      state.textContent = summary + (result && reason ? ` · ${REASONS[reason] || "请检查节点安装与连接状态"}` : "")
        + (!node.supported && ["current", "newer"].includes(node.reason) ? " · 尚未启用后台更新，需手动运行一次新版安装入口" : "")
        + (result?.errorCode ? ` · 错误码：${result.errorCode}` : "");
      row.dataset.state = result?.state || "idle";
      row.append(label, metadata, state); return row;
    }));
    $("storeUpgradeEmpty").hidden = rows.length !== 0;
    controls();
  }
  function schedule(active, current) {
    cancelTimer();
    if (!isActive() || active.signal.aborted || current !== epoch || getSession() !== active) return;
    timer = setTimeout(() => load(true), 2500);
  }
  async function load(poll = false, start = false) {
    const active = getSession(); if (!active || busy) return;
    cancelTimer();
    const current = ++epoch; busy = true; controls();
    if (!poll) $("storeUpgradeStatus").textContent = start ? "正在创建升级批次…" : "正在读取商店升级状态…";
    try {
      let result;
      if (start) result = await request(active, "store-node-upgrades", { method: "POST" });
      else if (poll && batch) result = await request(active, `store-node-upgrades/${encodeURIComponent(batch.batchId)}`);
      else result = await request(active, "store-node-upgrades");
      if (current !== epoch || active.signal.aborted || getSession() !== active) return;
      if (start || poll) batch = result;
      else { overview = result; batch = result.latestBatch; }
      if ((start || poll) && !isActive()) {
        const refreshed = await request(active, "store-node-upgrades");
        if (current !== epoch || active.signal.aborted || getSession() !== active) return;
        overview = refreshed;
      }
      render();
      $("storeUpgradeStatus").textContent = isActive()
        ? "批次进行中，页面会自动刷新。忙节点会等待已有任务与上传结束；网络暂断时等待确认。"
        : batch ? "批次已结束，请查看每个节点的结果。" : "状态已读取。发布新版不会自动安装，点击按钮才会创建批次。";
    } catch (error) {
      if (current === epoch && !active.signal.aborted && getSession() === active) {
        $("storeUpgradeStatus").textContent = error.message || "升级状态读取失败，请刷新重试。";
      }
    } finally {
      if (current === epoch) {
        busy = false; controls(); schedule(active, current);
      }
    }
  }
  $("storeUpgradeStart").addEventListener("click", () => {
    if (!$("storeUpgradeStart").disabled) load(false, true);
  });
  $("storeUpgradeRefresh").addEventListener("click", () => load());
  return {
    show() { $("storeUpgradeWorkspace").hidden = false; load(); },
    reset() {
      epoch++; cancelTimer(); busy = false; overview = undefined; batch = undefined;
      $("storeUpgradeWorkspace").hidden = true;
      $("storeUpgradeStatus").textContent = ""; render();
    },
  };
}
