# Connector 下载配置页

入口：`http://store-api.example.com/store-api/connector-setup/`。域名为占位，使用部署方提供的实际中心地址；HTTPS 入口也可使用。该页面属于独立 Store API，不依赖旧 Testflying 管理后台。

## 下载步骤

1. 打开页面后会自动加载节点，无需登录。公网访客可查看、新建节点并下载启用节点的安装包，包内含该节点的连接凭据。
2. 选择已有节点，或填写名称新建节点。默认下载 Windows x64，也支持 Linux x64、macOS Apple Silicon。
3. 选择需要启用的平台，可同时启用 iOS 和 Android。
4. iOS 填写团队 API Key 的 Issuer ID、Key ID，选择 Apple `.p8` 文件。文件名为 `AuthKey_XXX.p8` 时自动填写 Key ID；也可手动修改。
5. Android 选择 Google service account JSON，其邮箱、私钥等信息直接从文件读取。
6. 点击生成并下载，解压完整 ZIP，Windows 双击 `start.cmd`，Linux/macOS 执行 `./start.sh`。

不需要填写节点 ID、节点 Token、中心地址、密钥保存位置或应用清单。下载程序复用已发布二进制，不按节点重新编译。重新下载现有节点时保留其身份和中心映射；运行新包前应停止旧实例。

## 包内目录

```text
connector-windows-amd64-xxxxxxxx.zip
├── store-connector.exe
├── connector.json
├── start.cmd
├── testflying-prod-ca.crt
├── README.md
└── credentials/
    ├── apple-private-key.p8              # 启用 iOS 时包含
    └── google-service-account.json       # 启用 Android 时包含
```

`connector.json` 的 provider 自动引用 `credentials/` 下的相对路径。启动脚本先切换到程序目录，并按需设置信任中心公开 CA，不需要在配置中填写 Windows 绝对路径。Windows 启动脚本使用 CRLF 换行。

同一个节点的 provider 可同时包含 Apple 与 Google 凭据。应用映射仅保存在中心，包内不带应用清单；每次业务请求携带 platform 和 Bundle ID／包名。新增应用只更新中心映射，不用重新下载。禁用某个平台时，新包不包含该平台的凭据文件。

## 凭据处理

凭据文件在浏览器本地合并进 ZIP，不发送给中心、不进入中心数据库、日志或 GitHub Release。每个凭据文件最大 256 KiB。先检查选中的账号文件，再创建节点；下载失败后重试复用已创建的节点。

中心只接收节点名称，返回已发布程序和节点运行配置。网页的 ZIP 库随服务本地分发，不加载外部 CDN。页面设置 CSP、禁止嵌入、no-store 和 no-referrer。`/connector-setup/api/` 下的公开接口仅提供列表、创建和下载安装包。停用节点、轮换凭据和应用映射仍须通过需要管理员 Token 的管理接口。

文件格式检查不等于商店权限验证，商店 API Key／服务账号仍需具有对应应用权限。HTTP 页不提供传输加密，请使用可信网络；HTTPS 证书验证保持启用。下载后的 ZIP 含节点凭据及商店私钥，请私下保管，不要公开分享。

## 验证范围

安装包测试覆盖双平台同包、固定相对路径、身份不变、原 EXE 字节不变、错误文件、路径穿越、禁用平台不带凭据及 Windows 换行。浏览器验收覆盖直接加载、Key ID 自动识别、失败重试、真实 Windows 程序下载、刷新清理、移动端和凭据不上传。Windows 程序未在真实 Windows 硬件上运行；真实 Apple／Google 账号权限仍由实际接入后验证。
