export function bindLogs(request, getSession) {
  const $ = (id) => document.getElementById(id);
  let cursor;
  let filters;
  let sequence = 0;
  let busy = false;
  let downloading = false;
  let downloadEpoch = 0;
  let logNodes = [];

  function selectedNode() { return logNodes.find((node) => node.connectorId === $("nodeLogSelect").value); }
  function updateDownload() {
    const node = selectedNode();
    $("nodeLogDownload").disabled = downloading || !node?.enabled || !node?.logsAvailable;
    if (!downloading) $("nodeLogStatus").textContent = !node ? "暂无商店节点" : node.logsAvailable ? "可以读取最新日志文件。" : node.connected ? "该节点需安装最新升级包后才能读取日志。" : "节点未连接，暂时无法读取日志。";
  }
  function setNodes(nodes) {
    const selected = $("nodeLogSelect").value;
    logNodes = nodes.filter((node) => node.kind !== "screenshot");
    $("nodeLogSelect").replaceChildren(...logNodes.map((node) => new Option(`${node.name} · ${node.connected ? "已连接" : "未连接"}${node.logsAvailable ? "" : " · 需升级"}`, node.connectorId)));
    if (!logNodes.length) $("nodeLogSelect").add(new Option("暂无商店节点", ""));
    if (logNodes.some((node) => node.connectorId === selected)) $("nodeLogSelect").value = selected;
    updateDownload();
  }
  async function download() {
    const active = getSession(); const node = selectedNode();
    if (!active || !node || downloading || $("nodeLogDownload").disabled) return;
    const current = ++downloadEpoch;
    downloading = true; updateDownload(); $("nodeLogStatus").textContent = "正在从节点读取日志…";
    try {
      const bytes = await request(active, `connectors/${encodeURIComponent(node.connectorId)}/logs/latest`, { binary: true });
      if (downloadEpoch !== current || active.signal.aborted) return;
      const url = URL.createObjectURL(new Blob([bytes], { type: "application/x-ndjson" }));
      const link = document.createElement("a"); link.href = url; link.download = `store-connector-${node.connectorId.slice(0, 8)}-latest.jsonl`;
      document.body.append(link); link.click(); link.remove(); setTimeout(() => URL.revokeObjectURL(url), 1000);
      $("nodeLogStatus").textContent = "最新节点日志已下载。";
    } catch (error) {
      if (downloadEpoch === current && !active.signal.aborted) $("nodeLogStatus").textContent = error.message || "节点日志下载失败";
    } finally {
      if (downloadEpoch === current) { downloading = false; const node = selectedNode(); $("nodeLogDownload").disabled = !node?.enabled || !node?.logsAvailable; }
    }
  }
  $("nodeLogSelect").addEventListener("change", updateDownload);
  $("nodeLogDownload").addEventListener("click", download);
  $("nodeLogRefresh").addEventListener("click", async () => {
    const active = getSession(); if (!active) return;
    $("nodeLogRefresh").disabled = true;
    try { const result = await request(active, "connectors"); if (!active.signal.aborted) setNodes(result.connectors); }
    catch (error) { if (!active.signal.aborted) $("nodeLogStatus").textContent = error.message || "刷新失败"; }
    finally { if (!active.signal.aborted) $("nodeLogRefresh").disabled = false; }
  });

  function reset() {
    sequence++;
    busy = false;
    downloadEpoch++;
    downloading = false;
    setNodes([]);
    $("nodeLogRefresh").disabled = false;
    cursor = undefined;
    filters = undefined;
    $("logWorkspace").hidden = true;
    $("logWorkspace").open = false;
    $("logFilters").reset();
    $("logEntries").replaceChildren();
    $("logStatus").textContent = "";
    $("logSearch").disabled = false;
    $("logMore").hidden = true;
  }

  function selectedFilters() {
    const query = new URLSearchParams({ limit: "100" });
    for (const [id, key] of [["logRequestId", "requestId"], ["logIdentifier", "identifier"], ["logPlatform", "platform"], ["logConnectorId", "connectorId"]]) {
      const value = $(id).value.trim();
      if (value) query.set(key, value);
    }
    if ($("logErrorsOnly").checked) query.set("errorsOnly", "true");
    return query;
  }

  function render(entry) {
    const row = document.createElement("details");
    const summary = document.createElement("summary");
    summary.textContent = `${new Date(entry.timestampMs).toLocaleString()} · ${entry.event} · ${entry.status ?? "—"}${entry.identifier ? ` · ${entry.identifier}` : ""}`;
    const content = document.createElement("pre");
    content.textContent = JSON.stringify(entry, null, 2);
    row.append(summary, content);
    if (entry.requestId) {
      const trace = document.createElement("button");
      trace.type = "button";
      trace.className = "quiet";
      trace.textContent = "查看此请求全过程";
      trace.addEventListener("click", () => {
        if (busy) return;
        $("logFilters").reset();
        $("logRequestId").value = entry.requestId;
        load(false);
      });
      row.append(trace);
    }
    return row;
  }

  async function load(more) {
    const active = getSession();
    if (!active || busy) return;
    const current = ++sequence;
    busy = true;
    $("logSearch").disabled = true;
    $("logMore").disabled = true;
    $("logStatus").textContent = "正在查询…";
    if (!more) {
      cursor = undefined;
      filters = selectedFilters();
      $("logEntries").replaceChildren();
      $("logMore").hidden = true;
    }
    const query = new URLSearchParams(filters);
    if (more && cursor) query.set("before", cursor);
    try {
      const result = await request(active, `logs?${query}`);
      if (sequence !== current || active.signal.aborted) return;
      $("logEntries").append(...result.entries.map(render));
      cursor = result.nextCursor;
      $("logMore").hidden = !cursor;
      $("logStatus").textContent = result.entries.length ? "仅展示最近七天。展开事件可查看详情。" : "没有匹配日志。";
    } catch (error) {
      if (sequence === current && !active.signal.aborted) $("logStatus").textContent = error.message || "日志查询失败";
    } finally {
      if (sequence === current) {
        busy = false;
        $("logSearch").disabled = false;
        $("logMore").disabled = false;
      }
    }
  }

  $("logFilters").addEventListener("submit", (event) => { event.preventDefault(); load(false); });
  $("logMore").addEventListener("click", () => load(true));
  return { reset, setNodes, show() { $("logWorkspace").hidden = false; } };
}
