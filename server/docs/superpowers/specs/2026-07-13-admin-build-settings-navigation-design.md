# TestFlying 构建与设置入口重组设计

## 背景

TestFlying 管理后台已经具备应用级自动构建、构建记录、Build Runner、钉钉通知、
LLM 模型和运行环境配置，但入口仍按功能开发顺序平铺在一级导航中：

- “构建”只展示构建记录，不能从已接入应用直接发起构建。
- “构建节点”单独占用一级导航，与构建业务割裂。
- “LLM 配置”单独占用一级导航，后台没有统一设置入口。
- 钉钉等业务配置仍依赖环境变量，通知页只能展示配置教程。
- 数据库、MinIO/S3、Static Token 等基础设施配置没有统一的只读状态视图。

本设计重新组织入口，不改变现有构建调度、Runner 协议和商店同步能力。它取代
`2026-07-10-build-platform-delivery-closure-design.md` 中“通知页不新增独立设置页面、
钉钉凭据只允许环境变量配置”的管理界面约束；钉钉环境变量继续作为兼容回退。

## 目标

1. 一级导航只保留一个“构建”入口，构建应用、构建记录和节点配置在其内部组织。
2. 构建默认页展示已接入构建配置的应用，可选择环境和 Git ref 后直接触发构建。
3. 新增一级“设置”入口，统一承载通用业务配置、通知配置、LLM 配置和运行环境状态。
4. 业务配置支持后台保存并立即生效；基础设施环境变量只读、脱敏展示。
5. 删除旧一级页面和旧路由，页面切换保持在现有 React SPA 内，不发生整页刷新。

## 非目标

- 不允许后台修改数据库连接、MinIO/S3 凭据、Static Token、存储目录或 CORS。
- 不通过后台改写 Docker Compose、`.env` 或宿主机环境变量。
- 不改变 Runner 与中心服务的轮询、鉴权、制品上传和自动更新协议。
- 应用源码构建配置采用每个应用一份共享配置；开发环境和线上环境只在发起构建时选择。
  详细数据和接口约束见 `2026-07-14-single-app-build-setting-design.md`。
- 不在本次重组中引入新的通知渠道。

## 信息架构

### 一级导航

保留：

- 总览
- 上传
- 商店管理
- 商店评论
- 接口文档
- 构建
- 设备
- App 日志
- 通知
- 设置

删除一级入口：

- 构建节点
- LLM 配置

“通知”仍用于查看站内通知和投递结果；通知渠道配置移动到“设置”。

### 构建子页面

构建使用可链接的二级路由：

```text
/admin/builds/apps
/admin/builds/history
/admin/builds/runners
```

`/admin/builds` 默认显示应用构建子页面。旧 `/admin/build-runners` 路由直接删除，不提供
跳转页面。

二级导航固定为：

1. 应用构建
2. 构建记录
3. 节点配置

### 设置子页面

设置使用可链接的二级路由：

```text
/admin/settings/general
/admin/settings/notifications
/admin/settings/llm
/admin/settings/runtime
```

`/admin/settings` 默认显示通用设置子页面。旧 `/admin/llm-config` 路由直接删除，不提供
跳转页面。

二级导航固定为：

1. 通用设置
2. 通知设置
3. LLM 设置
4. 运行环境

## 构建工作区

### 应用构建

默认页面只展示已经保存共享构建配置的应用。每个应用项展示：

- 应用图标、名称、bundle identifier/package name 和平台
- Git 仓库和仓库子目录
- 共享构建配置是否完整
- 要求的 Runner labels
- 当前是否存在可匹配的在线节点
- 最近一次构建状态和时间

选中应用后在同一页面打开构建操作区：

- 环境：开发或线上；环境是本次构建参数，不切换配置
- Git ref：默认 `main`，允许输入 branch、tag 或 commit SHA
- 构建配置摘要：Git URL、子目录、产物类型和凭据引用名称
- “立即构建”按钮
- “编辑构建配置”入口，在当前页面打开构建配置弹窗

提交构建后不跳页。页面显示构建 ID、当前状态和“查看构建记录”入口。无匹配在线节点
不阻止创建排队任务，但必须在提交前清楚提示“当前无匹配在线节点”。

没有已接入应用时显示空状态，并提供“接入已有应用”入口。入口直接携带已有应用名称、包名和平台，
在当前页面完成一份共享构建配置；未配置应用不混入正常应用构建列表，也不跳转到商店管理。

### 构建记录

迁入现有构建列表，继续展示：

- 应用、版本、平台、来源和生命周期状态
- 失败摘要或人工处理提示
- 制品数量、安装 URL 和下载 URL
- 进入应用详情的操作

本次只调整页面归属和二级导航，不改变构建记录语义。

### 节点配置

迁入现有构建节点列表，并补齐已有后端能力的操作入口：

- 新增/预配节点
- 节点名称、ID、labels、平台能力和 LLM 适配器
- 生成一次性 Runner token
- 复制节点配置和安装说明
- 查看 Runner 版本、package-agent 版本、最近心跳和当前构建
- 展示节点是否落后于当前发布版本

一次性 token 只在预配成功响应中展示一次，刷新后不得再次返回。节点配置页面不得返回
其他节点的 token。

## 设置工作区

### 通用设置

第一版仅管理确实属于业务层、可以运行时生效的配置：

- Connector 默认地址模板 `connector_base_url_template`

账号级 Connector URL、凭据和 App 绑定仍留在开发者账号详情页，不提升为全局设置。

### 通知设置

从通知页迁入钉钉配置：

- Webhook URL
- 加签密钥
- 启用通知开关
- 请求超时时间
- 投递扫描间隔
- 固定触发状态说明：`failed`、`needs_human`
- 待发送和最终失败数量

页面提供“保存配置”和“检查配置”。检查会发送一条明确标注为“TestFlying 配置检查”
的钉钉测试消息，以验证签名、鉴权和网络可达性，但不创建站内通知或构建通知记录。
保存成功后新投递立即使用数据库配置，无需重启服务。关闭启用开关时保留已保存凭据，
停止新的钉钉投递；重新启用后继续使用原凭据。

通知列表页只保留通知筛选、通知内容和投递结果摘要，并提供“前往通知设置”入口。

### LLM 设置

完整迁入现有 LLM 配置页面，保留：

- OpenAI 兼容和 Claude 兼容协议
- 小米 MiMo 等预设
- Base URL、模型 ID、鉴权头和 API Key
- 评论分析、翻译、反馈分类的单模型互斥绑定

现有 LLM 表和 API 保持不变，只改变页面路由和导航归属。

### 运行环境

运行环境页面只读展示部署配置，按“服务”“存储”“安全”“Runner”分组。每项展示：

- 中文名称
- 环境变量名
- 当前来源：环境变量或默认值
- 安全值的脱敏摘要，或敏感值的“已配置/未配置”
- 是否需要重启后生效

至少覆盖：

- `TESTFLYING_DATABASE_URL`
- `TESTFLYING_PUBLIC_BASE_URL`
- `TESTFLYING_STORAGE_BACKEND`
- `TESTFLYING_STORAGE_ROOT`
- `TESTFLYING_S3_ENDPOINT_URL`
- `TESTFLYING_S3_PUBLIC_BASE_URL`
- `TESTFLYING_S3_BUCKET`
- `TESTFLYING_S3_ACCESS_KEY_ID`
- `TESTFLYING_S3_SECRET_ACCESS_KEY`
- `TESTFLYING_STATIC_TOKEN`
- `TESTFLYING_CORS_ALLOWED_ORIGINS`
- `TESTFLYING_ADMIN_USERNAME`
- `TESTFLYING_RUNNER_RELEASE_ROOT`

数据库 URL 中的用户名和密码、S3 凭据、Static Token 等敏感内容不得通过管理 API 返回。

## 配置存储和解析

新增 `system_settings` 键值表：

- `key`：稳定配置键，唯一
- `value`：配置值
- `is_secret`：是否敏感
- `updated_at`

第一版数据库配置键限定为：

- `connector_base_url_template`
- `dingtalk_enabled`
- `dingtalk_webhook_url`
- `dingtalk_secret`
- `dingtalk_timeout_seconds`
- `dingtalk_dispatch_interval_seconds`

有效值解析规则：

1. 数据库中存在有效业务配置时优先使用。
2. 数据库未配置时回退到现有环境变量。
3. 环境变量也未配置时使用当前代码默认值。

LLM 继续使用现有 `llm_profiles` 和 `llm_feature_bindings`，不迁入 `system_settings`。

钉钉投递循环每轮获取一次有效通知配置，确保后台保存后无需重启。Connector 地址生成逻辑
在每次使用时读取有效通用配置。设置 API 不修改启动时固定的 `Settings` 对象。

## API 设计

### 构建

新增：

- `GET /admin/api/builds/apps`：返回已接入构建应用、尚未接入的已有应用、共享配置、节点匹配状态和最近构建。

复用：

- `POST /admin/api/apps/{appId}/builds`：触发构建。
- `GET /admin/api/builds`：构建记录。
- `GET /admin/api/build-runners`：节点列表。
- `POST /admin/api/build-runners/provision`：预配节点并返回一次性 token。

### 设置

新增：

- `GET /admin/api/settings`：返回通用设置、通知设置和只读运行环境状态。
- `PUT /admin/api/settings/general`：保存通用业务配置。
- `PUT /admin/api/settings/notifications`：保存通知业务配置。
- `POST /admin/api/settings/notifications/check`：发送钉钉配置检查消息。

LLM 继续复用现有 `/admin/api/llm-config/*` API。

所有写操作返回更新后的页面状态。密钥字段留空表示不修改，响应仅返回是否已配置和脱敏
摘要，不返回原始值。

## 错误处理和审计

- 构建参数无效、应用未保存共享配置或 Git ref 为空时返回明确的字段级错误。
- 构建已创建但暂时没有匹配节点时返回排队状态，不伪装成失败。
- 通知设置校验失败时不覆盖已有配置。
- 数值配置必须是正数，并沿用当前超时和扫描间隔默认值。
- 所有设置写操作记录现有审计日志，只记录配置键和操作者，不记录密钥值。
- 前端保存和检查期间显示局部 loading，禁止重复提交。

## 前端组件边界

新增页面容器：

- `BuildWorkspacePage`：负责构建二级路由和共享导航。
- `BuildAppsPage`：负责在当前页面接入已有应用、维护构建配置、选择已接入应用和触发构建。
- `BuildHistoryPage`：承载现有构建列表。
- `BuildRunnersPage`：迁入节点配置子路由并增加预配操作。
- `SettingsPage`：负责设置二级路由和共享导航。
- `GeneralSettingsPage`
- `NotificationSettingsPage`
- `RuntimeSettingsPage`

现有 `LlmConfigPage` 作为 `SettingsPage` 的 LLM 子页面复用。现有 `BuildsPage` 的记录列表
逻辑移动为 `BuildHistoryPage`，避免单个页面同时承担路由、应用选择、记录和节点配置。

## 测试策略

后端测试覆盖：

- 一级导航不再返回 `build-runners` 和 `llm-config`，新增 `settings`。
- 已接入构建应用筛选、共享配置和节点匹配状态。
- 构建触发成功、未配置应用和无匹配节点排队。
- 业务设置数据库优先、环境变量回退和默认值回退。
- 钉钉配置保存、检查、热生效和失败不覆盖。
- 运行环境 API 对数据库 URL、S3 密钥和 Static Token 的脱敏。
- 一次性 Runner token 仅出现在预配响应中。

前端测试覆盖：

- 构建和设置子页面在 SPA 内切换，不触发 document navigation。
- 旧构建节点和 LLM 一级路由不再注册，也不再出现在导航数据中。
- 应用选择、环境选择、Git ref 输入和构建提交。
- 无匹配节点提示和提交后的构建状态反馈。
- 节点预配弹窗、一次性 token 展示和复制入口。
- 设置保存、密钥留空不修改、通知检查和错误反馈。
- 运行环境敏感值不出现在页面文本中。

## 验收标准

- 一级导航中不再出现“构建节点”和“LLM 配置”，出现“设置”。
- 打开“构建”默认能看到已接入应用，并能在同页触发测试或线上构建。
- 构建记录和节点配置均可通过构建二级导航访问。
- 设置页能保存 Connector 模板和钉钉配置，LLM 功能完整可用。
- 运行环境页能确认部署状态，但不能修改基础设施变量或读取敏感值。
- 旧构建节点和 LLM 一级页面及路由已经删除。
- 后台导航和子页面切换无整页刷新、无明显布局跳动。
- 后端、前端、迁移、Docker 构建和 GitHub Actions 全部通过。
