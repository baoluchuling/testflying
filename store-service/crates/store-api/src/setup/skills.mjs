export function bindScreenshotSkill(request, getSession) {
  const $ = (id) => document.getElementById(id);
  let state, busy = false, epoch = 0;
  function controls() {
    for (const control of $("screenshotSkillWorkspace").querySelectorAll("input, button, select")) control.disabled = busy;
    $("screenshotSkillActivate").disabled = busy || !state || !$("screenshotSkillVersion").value || $("screenshotSkillVersion").value === state.active?.version;
  }
  function render() {
    const active = state?.active;
    $("screenshotSkillActive").textContent = active
      ? `当前技能：${active.name}\n版本：${active.version}\nSHA-256：${active.sha256}\n文件：${active.files} · 包大小：${active.bytes} 字节\n发布于：${new Date(active.publishedAt * 1000).toLocaleString()}`
      : "尚未发布截图 Skill。";
    $("screenshotSkillVersion").replaceChildren(...(state?.versions || []).map((item) => new Option(`${item.name} · ${item.version.slice(0, 12)} · ${new Date(item.publishedAt * 1000).toLocaleString()}`, item.version)));
    if (active) $("screenshotSkillVersion").value = active.version;
    controls();
  }
  async function perform(action, message) {
    const active = getSession(); if (!active || busy) return;
    const current = ++epoch; busy = true; controls();
    $("screenshotSkillStatus").textContent = "正在处理 Skill…";
    try {
      const result = await action(active);
      if (current !== epoch || active.signal.aborted) return;
      state = result; render(); $("screenshotSkillStatus").textContent = message;
    } catch (error) {
      if (current === epoch && !active.signal.aborted) $("screenshotSkillStatus").textContent = error.message || "Skill 操作失败，请重试。";
    } finally { if (current === epoch) { busy = false; controls(); } }
  }
  $("screenshotSkillForm").addEventListener("submit", (event) => {
    event.preventDefault();
    const file = $("screenshotSkillFile").files[0];
    if (!file || !/\.zip$/i.test(file.name) || file.size === 0 || file.size > 2 * 1024 * 1024) {
      $("screenshotSkillStatus").textContent = "请选择不超过 2 MiB 的 Skill ZIP。"; return;
    }
    perform(async (active) => {
      const bytes = new Uint8Array(await file.arrayBuffer());
      active.signal.throwIfAborted(); $("screenshotSkillFile").value = "";
      return request(active, "screenshot-skill", {method:"POST",body:bytes,rawBody:true});
    }, "Skill 已发布，后续开始的任务会先更新并显式加载。运行中的任务继续使用原版本。");
  });
  $("screenshotSkillVersion").addEventListener("change", controls);
  $("screenshotSkillActivate").addEventListener("click", () => {
    const version = $("screenshotSkillVersion").value;
    if (version) perform((active) => request(active, "screenshot-skill", {method:"PUT",body:{version}}), "已切换 Skill 版本，后续开始的任务使用此版本。");
  });
  $("screenshotSkillRefresh").addEventListener("click", () => perform((active) => request(active, "screenshot-skill"), "Skill 版本已刷新。"));
  return {
    show() { $("screenshotSkillWorkspace").hidden = false; perform((active) => request(active, "screenshot-skill"), "Skill 版本已读取。"); },
    reset() {
      epoch++; busy = false; state = undefined;
      $("screenshotSkillWorkspace").hidden = true; $("screenshotSkillWorkspace").open = false;
      $("screenshotSkillForm").reset(); $("screenshotSkillStatus").textContent = ""; render();
    },
  };
}
