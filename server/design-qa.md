# testflying 管理后台 UI QA

日期：2026-06-27

## 视觉基准

- Demo：`/Users/admin/ai_project/tmp/testflying-design-preview/admin-store-settings-layout-demo.html`
- 参考截图：`/tmp/testflying-ui-qa/reference-demo-latest.png`

## 本地验证页面

- 默认商店页：`/tmp/testflying-ui-qa/qa-store-latest2.png`
- 营销页面：`/tmp/testflying-ui-qa/qa-marketing-latest2.png`
- 商店连接：`/tmp/testflying-ui-qa/qa-connection-latest2.png`
- App 日志连接态：`/tmp/testflying-ui-qa/qa-app-logs-connected-latest3.png`

## 检查结果

- 全局顶栏、页面标题区、动作按钮和健康状态区域已按 demo 的横向结构对齐。
- 默认商店页恢复 demo 的左侧 232px 商店设置导航、右侧默认商店页头部、四个锚点入口和纵向编辑区。
- 营销页面和商店连接使用同一商店管理外层框架，不再残留旧的商店元数据 tab 结构。
- App 日志连接后隐藏大标题区，连接卡与设备卡压缩到第一行，日志流区域扩大，黑底日志行改为紧凑列式展示。

## 结论

final result: passed
