export function bindLogs(request, getSession) {
  const $ = (id) => document.getElementById(id);
  let cursor;
  let filters;
  let sequence = 0;
  let busy = false;

  function reset() {
    sequence++;
    busy = false;
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
  return { reset, show() { $("logWorkspace").hidden = false; } };
}
