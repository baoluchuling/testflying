// API versions come from confirmed encrypted registrations. Offline values are historical.
export function versionLabel(node) {
  const latest = node.latestConnectorVersion || "未知";
  const reported = node.connectorVersion || "未知";
  const states = {current: "已是最新", outdated: "需要升级", newer: "高于中心发布版本", unknown: "需升级确认"};
  const state = states[node.versionStatus] || states.unknown;
  return `${node.connected ? "当前版本" : "最后上报版本"} ${reported} · 最新版本 ${latest} · ${node.connected ? state : `离线未核验（${state}）`}`;
}

export function nodeLabel(node) {
  return `${node.name} · ${node.enabled ? (node.connected ? "已连接" : "未连接") : "已停用"} · ${versionLabel(node)}`;
}
