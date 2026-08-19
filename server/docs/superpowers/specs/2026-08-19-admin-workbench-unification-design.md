# 管理后台工作台布局统一设计

## 背景

2026-07-07 的商店页面工作台化改造（见 `tmp/testflying-design-preview/admin-store-settings-layout-demo.html`
与当日会话决策）确立了紧凑工作台规范，并已落地到商店管理列表、商店评论、应用商店
管理详情三个页面内部（`compact-page / compact-context / compact-body` 结构，
`admin.css` 3323 行起的样式系统）。

但该规范只覆盖了商店相关页面：

- 全局壳 `AdminApp.tsx` 仍无条件渲染 `page-title-row`（eyebrow + 34px H1 +
  summary），仅 `app-logs` 和 `uploads` 两个路由通过 `data-route` 选择器隐藏。
- 设备、通知、设置、总览页面完全没有工作台化，进入后先看到大标题再看到内容，
  与 7-07 决策"高频操作页首屏直接进入内容工作区"不一致。

本设计把工作台规范推广到整个管理后台，不改任何业务行为。

## 规范（沿用 7-07 决策）

1. 高频操作页不显示全局大标题区；页面标题只出现在紧凑上下文工具栏里。
2. 上下文工具栏（48px）：左侧当前对象与关键计数；右侧主操作按钮。
3. 首屏直接展示主体内容；统计降级为工具栏或列头的 inline 计数。
4. 内容区固定高度、内部滚动：`compact-page` 以负 margin 逃逸 `admin-main`
   内边距，`height: calc(100vh - 56px)`。
5. 列头 48px，列内 padding 12/14px，外 gutter 24px，1600×900 无溢出。

## 改动范围

### 壳层

- `AdminApp.tsx` 删除 `page-title-row` 段与 `routeTitles` 依赖；页面标题改由
  各页面自己的 compact 工具栏承载。
- `admin.css` 删除 `.page-title-row` 系列规则与
  `.admin-shell[data-route="app-logs"]/.admin-shell[data-route="uploads"]` 的
  隐藏规则（壳层标题移除后二者失效）。
- `routes.tsx` 的 `routeTitles` 导出如无其他引用则一并移除。

### 页面工作台化

- 总览（DashboardPage）：compact 工具栏（总览 + 关键指标 + 上传入口），
  主体保持统计卡 + 快捷入口 + 最近构建/通知双栏，内容区内部滚动。
- 设备（DevicesPage）：compact 工具栏（设备 / N 台设备 / 刷新），表格主体。
- 通知（NotificationsPage）：compact 工具栏（通知 / N 条记录 / 管理通知渠道），
  类型筛选与通知列表保持现有行为（URL 筛选参数不变）。
- 设置（SettingsPage）：compact 工具栏（系统设置 + 说明），二级导航与四个
  子视图（通用/通知/LLM/运行环境）逻辑不变。
- 404 页保留自有标题结构，不依赖壳层大标题。

### 不改的部分

- 商店三个页面、构建工作台、App 日志、上传页已符合规范，不动。
- 接口文档页（ApiDocsPage）保留现有带内边距布局，仅失去壳层大标题。
- 所有数据加载、路由、URL 参数、API 调用行为不变。

## 验收

1. 任一后台页面首屏不再出现 `INTERNAL DISTRIBUTION` eyebrow 与 34px H1。
2. 设备/通知/设置/总览页面结构与商店页面一致：48px 工具栏 + 首屏内容。
3. 现有前端测试全部通过；`tsc`、lint、生产构建通过。
4. 服务端测试回归通过（SPA 产物重新构建后）。
