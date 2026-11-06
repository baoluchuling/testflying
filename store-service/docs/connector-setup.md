# Connector 下载配置页

入口为 `https://store-api.example.com/store-api/connector-setup/`。域名为占位，使用部署方提供的实际中心地址。该页面由独立 Store API 提供。

## 下载步骤

1. 在 HTTPS 页面输入 Store API 管理员 Token 解锁。解锁后可查看节点、新建节点、下载安装包和管理接入 Token。安装包内含该节点的连接凭据。
2. 选择已有节点，或填写名称新建节点。默认下载 Windows x64，也支持 Linux x64、macOS Apple Silicon。
3. 选择需要启用的平台，可同时启用 iOS 和 Android。
4. iOS 填写团队 API Key 的 Issuer ID、Key ID，选择 Apple `.p8` 文件。文件名为 `AuthKey_XXX.p8` 时自动填写 Key ID；也可手动修改。
5. Android 选择 Google service account JSON，其邮箱、私钥等信息直接从文件读取。
6. 下载并解压完整 ZIP。Windows 双击 `install.cmd` 并确认 UAC，macOS 在解压目录运行 `sh ./install.sh` 并完成管理员授权。两者安装完成后立即启动，并在系统开机时自动运行。Linux 运行 `./start.sh`。

不需要填写节点 ID、节点 Token、中心地址、密钥保存位置或应用清单。下载程序复用已发布二进制，不按节点重新编译。重新下载现有节点时保留其身份和中心映射；运行新包前应停止旧实例。

## 覆盖升级

1. 在下载页选择已安装的节点和原安装系统，勾选「仅升级程序」，下载 ZIP。
2. 已通过安装脚本安装的节点可在任意新目录解压，然后再次运行安装入口。安装器停掉原后台进程、更换程序并重新启动，原 `connector.json` 和商店凭据保留。
3. 从旧版手动运行包迁移时，先停止旧进程，把升级 ZIP 解压覆盖到旧目录，保留原 `connector.json` 与 `credentials/`，再运行安装入口。

每台机器安装一个后台节点。Windows 使用计划任务「Testflying Connector」，配置在 `%ProgramFiles%\Testflying Connector\data`。macOS 使用系统 LaunchDaemon `com.testflying.connector`，配置在 `/Library/Application Support/Testflying Connector/data`。暂停自启动可在 Windows 任务计划程序中结束并禁用该任务，macOS 执行 `sudo launchctl disable system/com.testflying.connector` 后再执行 `sudo launchctl bootout system/com.testflying.connector`。

## 包内目录

```text
connector-windows-amd64-xxxxxxxx.zip
├── store-connector.exe
├── connector.json
├── install.cmd
├── install.ps1
├── start.cmd
├── testflying-prod-ca.crt
├── README.md
└── credentials/
    ├── apple-private-key.p8              # 启用 iOS 时包含
    └── google-service-account.json       # 启用 Android 时包含
```

`connector.json` 的 provider 自动引用 `credentials/` 下的相对路径。启动脚本先切换到程序目录，并按需设置信任中心公开 CA，不需要在配置中填写 Windows 绝对路径。Windows 启动脚本使用 CRLF 换行。

同一个节点的 provider 可同时包含 Apple 与 Google 凭据。应用映射仅保存在中心，包内不带应用清单；每次业务请求携带 platform 和 Bundle ID／包名。新增应用只更新中心映射，不用重新下载。禁用某个平台时，新包不包含该平台的凭据文件。

## 本机平台范围

在商店 Connector 的本机 `connector.json` 顶层设置 `platformMode`，保存后重启 Connector。

| 值 | 允许的请求 |
|---|---|
| `all` | iOS 和 Android，旧配置省略此项时采用此值 |
| `ios` | 仅 iOS |
| `android` | 仅 Android |

单平台模式只加载对应的商店凭据。Connector 会以 `422 platform_not_supported` 拒绝其他平台的商店操作和图片上传。此设置保存在本机，修改已有节点时可保留原节点身份和绑定。

## 凭据处理

凭据文件在浏览器本地合并进 ZIP，不发送给中心、不进入中心数据库、日志或 GitHub Release。每个凭据文件最大 256 KiB。先检查选中的账号文件，再创建节点；下载失败后重试复用已创建的节点。

中心接收节点名称，返回已发布程序和节点运行配置。网页从本地加载 ZIP 库，并设置 CSP、禁止嵌入、no-store 和 no-referrer。页面和静态资源可公开访问，`/connector-setup/api/` 下的接口要求 HTTPS。用管理员 Token 登录一次后，浏览器通过 HttpOnly 会话 Cookie 在刷新时恢复登录。会话最长八小时，中心重启或点击「锁定管理」会使会话失效。刷新或锁定会清除表单、选中的文件和已显示的凭据。停用节点、轮换凭据和应用映射使用管理员 Bearer Token 调用管理 API。

商店 API Key／服务账号需具有对应应用权限。保持 HTTPS 证书验证启用。下载后的 ZIP 含节点凭据及商店私钥，请私下保管。

## 验证范围

安装包测试覆盖双平台同包、固定相对路径、身份不变、原 EXE 字节不变、错误文件、路径穿越、禁用平台不带凭据及 Windows 换行。浏览器验收覆盖 Key ID 自动识别、失败重试、真实 Windows 程序下载、刷新恢复会话及清除表单文件、锁定后重新登录、移动端和凭据不上传。Windows 程序的硬件验证及真实 Apple／Google 账号权限由实际接入时验证。
