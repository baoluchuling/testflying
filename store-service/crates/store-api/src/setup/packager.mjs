export const CREDENTIAL_LIMIT = 256 * 1024;
const ARCHIVE_LIMIT = 160 * 1024 * 1024;
const TARGETS = new Set(["windows-amd64", "linux-amd64", "darwin-arm64"]);
const encoder = new TextEncoder();

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

export async function buildInstaller(installerBytes, { target, account }, assets, JSZip) {
  if (!TARGETS.has(target)) throw new Error("不支持的安装平台");
  if (!(installerBytes instanceof Uint8Array) || !installerBytes.length || installerBytes.length > ARCHIVE_LIMIT) {
    throw new Error("安装包大小无效");
  }
  const validated = validateAccount(account);
  const binary = target === "windows-amd64" ? "store-connector.exe" : "store-connector";
  const launcher = target === "windows-amd64" ? "start.cmd" : "start.sh";
  const allowed = new Set([binary, launcher, "connector.json", "README.md", "testflying-prod-ca.crt"]);
  let zip;
  try { zip = await JSZip.loadAsync(installerBytes); }
  catch { throw new Error("中心返回的安装包无法读取"); }
  let total = 0;
  for (const item of Object.values(zip.files)) {
    const size = item._data?.uncompressedSize;
    if (item.dir || !allowed.has(item.name) || (item.unsafeOriginalName && item.unsafeOriginalName !== item.name) ||
        !Number.isSafeInteger(size) || size < 0 || size > (item.name === binary ? 128 * 1024 * 1024 : 1024 * 1024)) {
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
  const config = {};
  for (const key of ["connectorId", "token", "serverUrl", "messageKey", "uploadKey", "baseUrl", "listenAddr", "requestTimeoutSeconds"]) {
    if (Object.hasOwn(source, key)) config[key] = source[key];
  }
  config.provider = validated.provider;
  zip.file("connector.json", JSON.stringify(config, null, 2) + "\n", { unixPermissions: 0o600 });
  for (const file of validated.files) zip.file(file.path, file.bytes, { unixPermissions: 0o600 });
  if (!(assets.caBytes instanceof Uint8Array) || !new TextDecoder().decode(assets.caBytes).includes("BEGIN CERTIFICATE")) {
    throw new Error("中心 CA 证书不可用");
  }
  let script = launcher === "start.cmd" ? assets.windowsLauncher : assets.unixLauncher;
  if (typeof script !== "string" || !script.includes("SSL_CERT_FILE") || !script.includes("connector.json")) {
    throw new Error("启动脚本不可用");
  }
  if (launcher === "start.cmd") script = script.replace(/\r?\n/g, "\r\n");
  zip.file("testflying-prod-ca.crt", assets.caBytes, { unixPermissions: 0o644 });
  zip.file(launcher, script, { unixPermissions: 0o700 });
  zip.file("README.md", "# Connector\n\n解压整个安装包后运行 " + launcher +
    "。凭据已经放在 credentials 目录，路径已经写入 connector.json。\n" +
    "同一个节点可处理 iOS 和 Android，应用映射只在中心维护。\n" +
    "本包包含节点认证资料及商店私钥，请只保存在可信设备，不要上传公共仓库或转发。\n", { unixPermissions: 0o600 });
  const bytes = await zip.generateAsync({ type: "uint8array", compression: "DEFLATE", platform: "UNIX" });
  return { bytes, connectorId: config.connectorId, fileName: `connector-${target}-${config.connectorId.slice(0, 8)}.zip` };
}
