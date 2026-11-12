export function bindScreenshotSettings(request, getSession) {
  const $ = (id) => document.getElementById(id);
  const form = $("screenshotSettingsForm");
  const workspace = $("screenshotSettingsWorkspace");
  const fields = {
    region: "ossRegion", bucket: "ossBucket", endpoint: "ossEndpoint", prefix: "ossPrefix",
    publicBaseUrl: "ossPublicBaseUrl", accessKeyId: "ossAccessKeyId",
  };
  const secrets = ["ossAccessKeySecret", "ossSecurityToken", "screenshotNotificationToken"];
  let settings;
  let epoch = 0;
  let busy = false;
  const clearSecrets = () => { for (const id of secrets) $(id).value = ""; };

  function validate() {
    const enabled = $("ossEnabled").checked;
    for (const key of ["region", "bucket", "endpoint", "accessKeyId"]) $(fields[key]).required = enabled;
    $("ossAccessKeySecret").required = (enabled && !settings?.oss.accessKeySecretConfigured)
      || Boolean(settings && $("ossAccessKeyId").value.trim() !== settings.oss.accessKeyId);
    for (const id of ["ossEndpoint", "ossPublicBaseUrl", "screenshotNotificationUrl"]) {
      const field = $(id);
      field.setCustomValidity("");
      if (!field.value.trim()) continue;
      try {
        const url = new URL(field.value.trim());
        if (url.protocol !== "https:" || url.username || url.password || url.search || url.hash
          || (id === "ossEndpoint" && url.pathname !== "/")) throw new Error();
      } catch {
        field.setCustomValidity(id === "ossEndpoint" ? "请填写完整 HTTPS Bucket 域名或 CNAME，不含路径和查询参数。" : "请填写不含账号信息、查询参数或片段的 HTTPS URL。");
      }
    }
  }

  function renderBusy() {
    for (const control of form.querySelectorAll("input, button")) control.disabled = busy;
    $("screenshotSettingsSave").disabled = busy || !settings;
    $("screenshotSettingsSave").textContent = busy ? "正在处理…" : "保存截图设置";
    form.setAttribute("aria-busy", String(busy));
  }

  function populate(value) {
    settings = value;
    $("ossEnabled").checked = value.oss.enabled;
    for (const [key, id] of Object.entries(fields)) $(id).value = value.oss[key] ?? "";
    $("screenshotNotificationUrl").value = value.notification.url ?? "";
    for (const [id, configured] of [
      ["ossAccessKeySecret", value.oss.accessKeySecretConfigured],
      ["ossSecurityToken", value.oss.securityTokenConfigured],
      ["screenshotNotificationToken", value.notification.tokenConfigured],
    ]) $(id).placeholder = configured ? "已配置，留空保留" : "尚未配置";
    $("screenshotNotificationState").value = value.notification.status === "not_configured"
      ? "未配置完整地址与 Token；不影响截图执行" : "已配置；任务结束后自动回传 Studio";
    clearSecrets();
    validate();
  }

  async function perform(save = false) {
    const active = getSession();
    if (!active || busy || (save && !settings)) return;
    if (save) {
      validate();
      if (!form.reportValidity()) return;
    }
    const current = ++epoch;
    let body;
    if (save) {
      const oss = { enabled: $("ossEnabled").checked };
      for (const [key, id] of Object.entries(fields)) oss[key] = $(id).value.trim();
      if ($("ossAccessKeySecret").value) oss.accessKeySecret = $("ossAccessKeySecret").value;
      if ($("ossSecurityToken").value) oss.securityToken = $("ossSecurityToken").value;
      const notification = { url: $("screenshotNotificationUrl").value.trim() };
      if ($("screenshotNotificationToken").value) notification.token = $("screenshotNotificationToken").value;
      body = { oss, notification };
    }
    busy = true;
    renderBusy();
    $("screenshotSettingsStatus").textContent = save ? "正在保存截图设置…" : "正在读取截图设置…";
    try {
      const pending = request(active, "screenshot-settings", save ? { method: "PUT", body } : {});
      clearSecrets();
      body = undefined;
      const result = await pending;
      if (current !== epoch || active.signal.aborted || getSession() !== active) return;
      populate(result);
      $("screenshotSettingsStatus").textContent = save
        ? "截图设置已保存。上传与结果通知将使用最新配置。"
        : "截图设置已刷新，密钥不回显。";
    } catch {
      if (current === epoch && !active.signal.aborted && getSession() === active) {
        $("screenshotSettingsStatus").textContent = save
          ? "截图设置保存失败，请检查配置后重试；如更换密钥，请重新填写。"
          : "截图设置读取失败，请刷新重试。";
      }
    } finally {
      if (current === epoch) { clearSecrets(); busy = false; validate(); renderBusy(); }
    }
  }

  form.addEventListener("input", validate);
  form.addEventListener("submit", (event) => { event.preventDefault(); perform(true); });
  $("screenshotSettingsRefresh").addEventListener("click", () => perform());
  return {
    reset() {
      epoch++;
      settings = undefined;
      busy = false;
      workspace.hidden = true;
      workspace.open = false;
      form.reset();
      clearSecrets();
      for (const id of secrets) $(id).placeholder = "";
      $("screenshotSettingsStatus").textContent = "";
      validate();
      renderBusy();
    },
    show() { workspace.hidden = false; perform(); },
  };
}
