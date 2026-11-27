export const CREDENTIAL_LIMIT = 256 * 1024;
const ARCHIVE_LIMIT = 160 * 1024 * 1024;
const TARGETS = new Set(["windows-amd64", "linux-amd64", "darwin-arm64"]);
const encoder = new TextEncoder();

export function archiveNodeName(name) {
  const safe = String(name || "").normalize("NFC")
    .replace(/[<>:"/\\|?*\p{Cc}\p{Cf}]/gu, "-").replace(/\s+/gu, "-");
  return Array.from(safe).slice(0, 40).join("").replace(/^[ .-]+|[ .-]+$/gu, "") || "node";
}

export function installationFiles(target) {
  if (target === "windows-amd64") return ["install.cmd", "install.ps1"];
  if (target === "darwin-arm64") return ["install.sh"];
  return [];
}

function text(bytes, label) {
  if (!(bytes instanceof Uint8Array) || !bytes.length || bytes.length > CREDENTIAL_LIMIT) {
    throw new Error(`${label}为空或超过 256 KiB`);
  }
  try { return new TextDecoder("utf-8", { fatal: true }).decode(bytes).replace(/^\uFEFF/, ""); }
  catch { throw new Error(`${label}不是有效的 UTF-8 文件`); }
}

function required(value, label) {
  if (typeof value !== "string" || !value.trim() || value.length > 200) {
    throw new Error(`请填写有效的${label}`);
  }
  return value.trim();
}

function pem(value, label) {
  if (!/^-----BEGIN (?:RSA )?PRIVATE KEY-----\s+[A-Za-z0-9+/=\s]+-----END (?:RSA )?PRIVATE KEY-----\s*$/.test(value.trim())) {
    throw new Error(`${label}不是有效的 PEM 私钥文件`);
  }
}

async function validateUpdateTrust(zip, source, target) {
  const entry = zip.file("update-trust.json");
  if (!entry) throw new Error("中心安装包缺少升级信任配置，请重新下载");
  let trust;
  try { trust = JSON.parse(await entry.async("string")); }
  catch { throw new Error("升级信任配置格式无效"); }
  if (trust?.protocol !== 1 || trust.nodeId !== source.connectorId || trust.target !== target ||
      typeof trust.serverUrl !== "string" || typeof trust.publicKey !== "string" ||
      !/^04[0-9a-fA-F]{128}$/.test(trust.publicKey)) throw new Error("升级信任配置与节点或系统不匹配");
  try {
    const server = new URL(trust.serverUrl), expected = new URL(source.serverUrl);
    if (!["http:", "https:"].includes(expected.protocol)) throw new Error();
    expected.protocol = "https:";
    if (server.protocol !== "https:" || server.username || server.password || server.search || server.hash ||
        expected.username || expected.password || expected.search || expected.hash ||
        server.href.replace(/\/$/, "") !== expected.href.replace(/\/$/, "")) throw new Error();
    const key = Uint8Array.from(trust.publicKey.match(/../g), (byte) => Number.parseInt(byte, 16));
    await globalThis.crypto.subtle.importKey("raw", key, { name: "ECDSA", namedCurve: "P-256" }, false, ["verify"]);
  } catch { throw new Error("升级信任配置的 HTTPS 中心地址或验证公钥无效"); }
}

export function validateAccount(input) {
  if (!input?.apple && !input?.google) throw new Error("请至少启用 iOS 或 Android");
  const provider = { mode: "live" };
  const files = [];
  if (input.apple) {
    const apple = input.apple;
    const content = text(apple.content, "Apple .p8 文件");
    pem(content, "Apple .p8 文件");
    if (content.includes("BEGIN RSA PRIVATE KEY")) throw new Error("Apple 需要 P-256 .p8 私钥");
    provider.appleIssuerId = required(apple.issuerId, " Issuer ID");
    provider.appleKeyId = required(apple.keyId, " Key ID");
    provider.applePrivateKeyPath = "credentials/apple-private-key.p8";
    files.push({ path: provider.applePrivateKeyPath, bytes: encoder.encode(content) });
  }
  if (input.google) {
    const content = text(input.google.content, "Google 服务账号 JSON");
    let account;
    try { account = JSON.parse(content); }
    catch { throw new Error("Google 服务账号文件不是有效 JSON"); }
    if (account?.type !== "service_account" || typeof account.client_email !== "string" ||
        !account.client_email.includes("@") || typeof account.private_key !== "string") {
      throw new Error("Google 文件需包含 service_account、client_email 和 private_key");
    }
    pem(account.private_key, "Google 服务账号私钥");
    provider.googleServiceAccountPath = "credentials/google-service-account.json";
    files.push({ path: provider.googleServiceAccountPath, bytes: encoder.encode(content) });
  }
  return { provider, files };
}

export async function buildInstaller(installerBytes, { target, account, upgrade = false, nodeName }, assets, JSZip) {
  if (!TARGETS.has(target)) throw new Error("不支持的安装平台");
  const installFiles = installationFiles(target);
  if (upgrade && !installFiles.length) throw new Error("所选平台不支持仅升级程序");
  if (!(installerBytes instanceof Uint8Array) || !installerBytes.length || installerBytes.length > ARCHIVE_LIMIT) {
    throw new Error("安装包大小无效");
  }
  const validated = upgrade ? null : validateAccount(account);
  const binary = target === "windows-amd64" ? "store-connector.exe" : "store-connector";
  const launcher = target === "windows-amd64" ? "start.cmd" : "start.sh";
  const allowed = new Set([binary, launcher, "connector.json", "update-trust.json", "README.md", "testflying-prod-ca.crt"]);
  let zip;
  try { zip = await JSZip.loadAsync(installerBytes); }
  catch { throw new Error("中心返回的安装包无法读取"); }
  let total = 0;
  for (const item of Object.values(zip.files)) {
    const size = item._data?.uncompressedSize;
    if (item.dir || !allowed.has(item.name) || (item.unsafeOriginalName && item.unsafeOriginalName !== item.name) ||
        !Number.isSafeInteger(size) || size < 0 || size > (item.name === binary ? 128 * 1024 * 1024 : item.name === "update-trust.json" ? 8192 : 1024 * 1024)) {
      throw new Error("安装包包含不允许的文件或路径");
    }
    total += size;
  }
  if (total > ARCHIVE_LIMIT || !zip.file(binary) || !zip.file("connector.json") || !zip.file(launcher)) {
    throw new Error("安装包缺少程序、配置或启动脚本");
  }
  // Check CRC only after enforcing declared decompression bounds.
  try { zip = await JSZip.loadAsync(installerBytes, { checkCRC32: true }); }
  catch { throw new Error("安装包完整性检查失败"); }
  let source;
  try { source = JSON.parse(await zip.file("connector.json").async("string")); }
  catch { throw new Error("中心安装配置格式无效"); }
  for (const key of ["connectorId", "token", "serverUrl", "messageKey", "uploadKey"]) {
    if (typeof source[key] !== "string" || !source[key]) throw new Error("中心安装配置不完整");
  }
  const { connectorId } = source;
  if (!/^[A-Za-z0-9_-]{1,128}$/.test(connectorId)) throw new Error("中心节点身份无效");
  await validateUpdateTrust(zip, source, target);
  if (upgrade) {
    zip.remove("connector.json");
    zip.file("node-id.txt", connectorId + "\n", { unixPermissions: 0o600 });
  } else {
    const config = {};
    for (const key of ["connectorId", "token", "serverUrl", "messageKey", "uploadKey", "baseUrl", "listenAddr", "requestTimeoutSeconds"]) {
      if (Object.hasOwn(source, key)) config[key] = source[key];
    }
    config.provider = validated.provider;
    config.platformMode = account.apple && account.google ? "all" : account.apple ? "ios" : "android";
    zip.file("connector.json", JSON.stringify(config, null, 2) + "\n", { unixPermissions: 0o600 });
    for (const file of validated.files) zip.file(file.path, file.bytes, { unixPermissions: 0o600 });
  }
  if (!(assets.caBytes instanceof Uint8Array) || !new TextDecoder().decode(assets.caBytes).includes("BEGIN CERTIFICATE")) {
    throw new Error("中心 CA 证书不可用");
  }
  const launcherText = assets.scripts?.[launcher];
  if (typeof launcherText !== "string" || !launcherText.includes("SSL_CERT_FILE") || !launcherText.includes("connector.json")) {
    throw new Error("启动脚本不可用");
  }
  for (const name of [launcher, ...installFiles]) {
    let script = assets.scripts[name];
    if (typeof script !== "string" || !script.trim()) throw new Error("安装脚本不可用");
    if (name.endsWith(".cmd") || name.endsWith(".ps1")) script = script.replace(/\r?\n/g, "\r\n");
    zip.file(name, script, { unixPermissions: 0o700 });
  }
  zip.file("testflying-prod-ca.crt", assets.caBytes, { unixPermissions: 0o644 });
  const entry = installFiles[0] ?? launcher;
  const instructions = upgrade
    ? "升级已安装节点时，解压后运行 " + entry + "，本机配置会保留。\n" +
      "如果旧版是手动运行的，把本包解压覆盖到旧目录，保留原 connector.json 和 credentials，再运行安装。\n"
    : "解压整个安装包后运行 " + entry + (installFiles.length ? "，确认系统授权后会立即启动并设置开机启动。\n" +
      "已有相同节点时，自动补充或更新本次提供的平台资料，未提供的平台继续保留。\n" : "。\n") +
      "凭据位于 credentials 目录，请只保存在可信设备。\n";
  const logGuide = "\n节点日志保存在 connector.json 同目录的 logs 文件夹，按小时分文件、最多保留七天。\n" +
    "Windows 安装后位于 C:\\Program Files\\Testflying Connector\\data\\logs；macOS 位于 /Library/Application Support/Testflying Connector/data/logs。\n" +
    "日志包含任务关联、上游调用步骤、状态、耗时和请求长度，不记录凭据、文案原文或图片。\n" +
    "\nWindows/macOS 旧商店节点必须手动运行一次本包安装入口，之后才支持中心后台按钮升级；发布新版不会自动安装。\n" +
    "update-trust.json 绑定本节点、系统、HTTPS 中心和升级验证公钥，不含签名私钥。请保留此文件并完整解压安装。\n" +
    "后台升级仅保留当前程序、上一健康版本和一个候选；成功确认后清理更老程序和已用 ZIP，半包/临时目录最多 24 小时，升级日志七天。\n" +
    "升级器不清理配置、credentials、业务 logs 或手工解压的旧目录；截图节点不参与商店后台升级。\n";
  zip.file("README.md", "# Connector\n\n" + instructions + logGuide, { unixPermissions: 0o600 });
  const bytes = await zip.generateAsync({ type: "uint8array", compression: "DEFLATE", platform: "UNIX" });
  return { bytes, connectorId, fileName: `connector-${target}-${archiveNodeName(nodeName)}-${connectorId.slice(0, 8)}${upgrade ? "-upgrade" : ""}.zip` };
}
