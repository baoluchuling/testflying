const $ = (id) => document.getElementById(id);

export function bindIntegrations(request, getSession) {
  const workspace = $("integrationWorkspace");
  const list = $("integrationList");
  const form = $("integrationIssue");
  const field = $("integrationToken");
  let activeOperation;

  function message(text) {
    $("integrationStatus").textContent = text;
  }

  function showToken(token = "") {
    field.value = token;
    $("integrationSecret").hidden = !token;
  }

  async function run(action) {
    const session = getSession();
    if (!session || activeOperation === session) return;
    activeOperation = session;
    try {
      await action(session);
    } catch (error) {
      if (!session.signal.aborted) message(error.message || "凭据操作失败");
    } finally {
      if (activeOperation === session) activeOperation = undefined;
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

  function renderList(integrations) {
    list.replaceChildren();
    for (const integration of integrations) {
      const row = document.createElement("li");
      row.append(document.createTextNode(`${integration.name} · 全部节点 `));
      const path = `integrations/${encodeURIComponent(integration.id)}`;
      row.append(actionButton("取回 Token", async (session) => {
        showToken((await request(session, path + "/token")).token);
        message("可以复制这枚接入 Token。");
      }));
      row.append(actionButton("撤销", async (session) => {
        if (!window.confirm(`撤销「${integration.name}」的 Token？`)) return;
        await request(session, path, { method: "DELETE" });
        showToken();
        await refresh(session);
        message("Token 已撤销。");
      }));
      list.append(row);
    }
  }

  async function refresh(session) {
    const result = await request(session, "integrations");
    renderList(result.integrations);
  }

  workspace.addEventListener("toggle", () => {
    if (workspace.open) run(refresh);
    else showToken();
  });

  form.addEventListener("submit", (event) => {
    event.preventDefault();
    run(async (session) => {
      const result = await request(session, "integrations", {
        method: "POST",
        body: { name: $("integrationName").value.trim() },
      });
      form.reset();
      await refresh(session);
      if (workspace.open) showToken(result.token);
      message("Token 已发放，可复制到该系统的后端。");
    });
  });

  $("integrationCopy").addEventListener("click", () => run(async (session) => {
    try {
      await navigator.clipboard.writeText(field.value);
      if (!session.signal.aborted) message("Token 已复制。");
    } catch {
      if (session.signal.aborted) return;
      field.focus();
      field.select();
      message("已选中 Token，请使用系统快捷键复制。");
    }
  }));

  return {
    show() {
      workspace.hidden = false;
    },
    reset() {
      showToken();
      form.reset();
      list.replaceChildren();
      workspace.open = false;
      workspace.hidden = true;
      message("");
    },
  };
}
