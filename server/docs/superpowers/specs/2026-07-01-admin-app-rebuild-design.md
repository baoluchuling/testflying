# testflying 管理后台重构设计

## 背景

当前 `/admin` 后台是 FastAPI + Jinja2 模板实现。页面切换依赖服务端整页渲染，`base.html` 中又叠加了大量原生 JavaScript 做局部增强。随着商店同步、评论分析、App 日志、上传进度、Connector 和多语言截图管理加入，后台已经超过了简单服务端模板页面的复杂度。

现状问题：

- 顶部导航和页内应用切换会重新加载或整块替换页面内容。
- 上传进度、App 日志连接、服务健康状态这类跨页面状态容易丢失。
- 多个页面的交互逻辑集中在 `templates/admin/base.html`，难以维护和测试。
- 商店管理和评论分析需要大量局部状态，继续用表单提交和模板重渲染会让体验和代码都变差。

## 目标

构建一套独立的管理后台 Admin App，让页面切换和页内切换都不触发整页刷新，同时保留 FastAPI 后端作为业务能力和数据能力的唯一来源。

完成后应满足：

- 顶部导航切换页面时不刷新浏览器页面。
- 评论页切应用、筛选、拉取最新评论、LLM 分析都局部更新。
- 商店管理页切模块、切同步项、编辑多语言、上传截图、同步确认都局部更新。
- 上传任务在切换页面时不中断，回到上传页仍能看到进度。
- App 日志连接状态不因页面切换丢失。
- 后端业务逻辑不搬到前端；前端只负责展示、输入和交互编排。
- 旧 Jinja 后台在迁移期保留为回滚入口。

## 非目标

- 不重写核心 FastAPI 服务。
- 不修改 Connector 协议。
- 不改变现有数据库模型，除非迁移某个页面时发现前端状态必须持久化。
- 不引入自动回复评论、自动修改商店文案等能力。
- 不一次性删除旧后台模板。

## 架构选择

采用 `Vite + React + TypeScript` 新建独立前端 Admin App，FastAPI 继续作为后端和部署容器。

目录结构：

```text
admin-web/
  package.json
  index.html
  vite.config.ts
  tsconfig.json
  src/
    main.tsx
    app/
      AdminApp.tsx
      routes.tsx
      apiClient.ts
      query.ts
    components/
      AppShell.tsx
      Button.tsx
      StatusBadge.tsx
      EmptyState.tsx
      Modal.tsx
    pages/
      StoreAppsPage.tsx
      StoreReviewsPage.tsx
      StoreManagementPage.tsx
      UploadPage.tsx
      AppLogsPage.tsx
      BuildsPage.tsx
      DevicesPage.tsx
      NotificationsPage.tsx
    styles/
      admin.css
```

FastAPI 挂载：

- `/admin` 返回新 Admin App 的 `index.html`。
- `/admin/assets/*` 返回 Vite build 后的静态资源。
- `/admin-legacy` 保留旧 Jinja 后台入口。
- `/admin/api/*` 提供新后台专用 JSON API。
- 现有公开 API，例如 `/v1/store-management/...`、`/uploads`、`/devices`，继续保留。

## 后端边界

新前端不直接使用 Jinja 模板上下文，统一使用 JSON API。

后端新增或整理三层：

- `src/testflying_api/admin_api/routes.py`：新后台 JSON 路由。
- `src/testflying_api/admin_api/schemas.py`：新后台请求和响应模型。
- 现有 service：继续承载业务逻辑，例如 `store_reviews.py`、`store_sync.py`、`upload_service.py`、`app_logs.py`。

规则：

- JSON API 不复制业务逻辑，只调用已有 service。
- HTML 旧路由和 JSON 新路由可以暂时并存。
- 写操作返回结构化结果和下一步状态，而不是返回整页 HTML。
- 所有错误返回统一 JSON：`{ "error": { "code": "...", "message": "...", "detail": ... } }`。

## 前端状态边界

全局状态：

- 当前路由。
- 服务健康状态。
- 上传任务状态。
- App 日志连接和设备状态。
- 全局 toast / error。

页面状态：

- 评论页：选中应用、评分筛选、评论列表、分析结果、拉取状态。
- 商店管理：选中 App、当前模块、当前同步项、当前语言、多语言展开状态、同步确认弹窗。
- 上传页：当前表单输入、上传进度、解析出的包信息。
- 普通列表页：筛选条件、列表数据、加载状态。

数据拉取策略：

- 页面进入时调用 `load()`。
- 写操作完成后刷新当前页面需要的状态，不刷新整个 Admin App。
- 对会被后台改变的状态，不只靠前端乐观更新；以服务端返回为准。

## 迁移顺序

### 阶段 1：新 Admin App 壳

交付：

- Vite/React/TypeScript 工程。
- 顶部导航、健康检查、全局错误、加载态。
- FastAPI 能服务新前端 build 产物。
- `/admin-legacy` 可访问旧后台。

验收：

- 打开 `/admin` 显示新后台壳。
- 切换顶部导航不触发整页刷新。
- `/admin-legacy` 仍能打开旧后台。

### 阶段 2：评论分析页

交付：

- 应用列表。
- 评论列表。
- 评分筛选。
- 拉取最新评论。
- LLM 分析。
- 分析结果和错误提示。

验收：

- 切应用不刷新整页。
- 拉取和分析按钮只局部更新。
- 空库首次拉取仍只拉一页 20 条。
- 有历史后遇到已存在同创建日期评论停止。

### 阶段 3：商店管理页

交付：

- 商店应用列表。
- 默认商店页。
- 营销页面。
- 商店连接。
- 多语言编辑。
- 截图上传预览和删除。
- 同步确认弹窗。

验收：

- 切商店模块不刷新整页。
- 同步前弹窗显示本次同步范围。
- 保存草稿和同步到商店都局部更新。

### 阶段 4：上传和 App 日志

交付：

- 上传 IPA/APK。
- 上传进度跨页面保留。
- 解析包信息展示。
- App 日志连接、设备列表、日志流、筛选。

验收：

- 上传中切页面不会中断。
- App 日志连接后切页面再回来仍保持连接状态。
- 连接上后日志区优先占用空间。

### 阶段 5：普通页面和收尾

交付：

- 首页、构建、设备、通知。
- API 文档入口。
- 旧后台下线计划。

验收：

- 主要后台功能都在新 Admin App 可用。
- 旧 Jinja 路由只作为 legacy 入口存在。
- CI、Docker、远程部署全部通过。

## 测试策略

后端：

- 保留现有 pytest。
- 新增 `/admin/api/*` 路由测试。
- 写操作测试必须覆盖成功、错误和状态刷新。

前端：

- 使用 `vitest` 测试纯函数和 API client。
- 使用 Playwright 做核心冒烟：
  - 顶部导航切换不整页刷新。
  - 评论页切应用不整页刷新。
  - 评论页拉取和分析局部更新。
  - 上传中切页状态保留。
  - App 日志连接状态保留。

构建：

- CI 增加 Node 安装、前端 lint/test/build。
- Docker builder 阶段先构建 `admin-web`，再打 Python wheel。
- Python package data 包含前端 build 产物。

## 风险与缓解

- 风险：新旧后台路由冲突。
  - 缓解：新后台使用 `/admin`，旧后台整体移动到 `/admin-legacy`，迁移期保留兼容跳转。

- 风险：业务逻辑重复。
  - 缓解：JSON API 只调用已有 service；发现 HTML 路由内嵌业务逻辑时先抽 service。

- 风险：前端状态和数据库状态不一致。
  - 缓解：写操作完成后统一刷新当前页面 state，不只做本地状态修改。

- 风险：一次性改动过大。
  - 缓解：按页面迁移，每阶段可独立测试和部署。

- 风险：部署缓存加载旧 JS。
  - 缓解：Vite 产物文件名带 hash，FastAPI 返回新 `index.html`。

## 回滚策略

- 迁移期保留 `/admin-legacy`。
- 如果新后台某页面异常，可以从新页面放置“打开旧版后台”入口。
- Docker 部署失败时回滚上一版镜像。
- 数据库不做破坏性迁移，降低回滚成本。

## 第一阶段完成标准

- `admin-web` 已搭建并可构建。
- `/admin` 使用新 Admin App。
- `/admin-legacy` 使用旧 Jinja 后台。
- CI 同时跑后端和前端验证。
- Docker 镜像包含前端构建产物。
- 至少评论页迁移完成前，旧评论页仍可在 legacy 中使用。
