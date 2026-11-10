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

Windows 安装器会显示五个阶段：检查路径和节点配置、保留或准备凭据、准备新版程序、更新后台任务、请求后台启动。最后的安装完成提示表示文件安装完成且已请求任务启动；它不代表中心已经确认连接。计划任务的 Running 状态也只代表任务在执行，连接成功以中心节点 connected 状态为准。

Windows 后台任务从安装的 `data` 目录执行，程序及 CA 证书位于同级 `program` 目录。启动脚本按自身绝对目录检查和设置 CA 路径，不依赖当前目录是否存在证书文件。此前程序在前台显式设置 SSL_CERT_FILE 后可以连接、后台未配置证书的安装，应重新下载最新升级包并运行 install.cmd。显式自定义的 SSL_CERT_FILE 仍保留。

每台机器安装一个后台节点。Windows 使用计划任务「Testflying Connector」，配置在 `%ProgramFiles%\Testflying Connector\data`。macOS 使用系统 LaunchDaemon `com.testflying.connector`，配置在 `/Library/Application Support/Testflying Connector/data`。暂停自启动可在 Windows 任务计划程序中结束并禁用该任务，macOS 执行 `sudo launchctl disable system/com.testflying.connector` 后再执行 `sudo launchctl bootout system/com.testflying.connector`。

## 自动补充平台或更新账号

Windows、macOS 的完整安装包可更新已安装的同一个节点，无需手动编辑 JSON：

1. 选择原来的节点和对应系统，不勾选「仅升级程序」。
2. 只勾选本次需要补充或更新的平台，填写该平台资料。例如已有安卓节点补 iOS，只需提供 Apple Issuer ID、Key ID 和 `.p8`；安卓本次不勾选，原安卓配置继续保留。
3. 下载完整安装包，停止旧的手动实例，再运行 `install.cmd` 或 `sh ./install.sh`。安装器会校验包与已有节点身份，准备合并结果后停止后台任务并应用配置，随后重启。

本次提供的平台凭据会更新，未提供平台的配置和凭据不变；节点 Token、连接密钥、服务器地址、监听设置及应用绑定保持原样。仅 iOS 增加安卓或仅安卓增加 iOS 时，本机 `platformMode` 自动扩展为 `all`；已有 `all` 不会因本次只提供一个平台而缩窄。首装按本次选择设为 `ios`、`android` 或 `all`。此流程用于补充或更新，取消勾选不会移除已安装平台；主动限制平台仍可按下文修改本机 `platformMode`。

输入错误、节点不匹配或缺少所选凭据会在停止旧服务前拒绝。后续安装失败会恢复旧配置、被更新凭据、程序和原任务状态。安装过程中断且恢复失败时保留恢复目录并提示，不自动删除不明状态。仅升级程序包不进入配置合并，继续保留本机配置的原始字节。凭据只在本机合并，不上传中心。Linux 手动运行包不提供这个自动安装流程。

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
