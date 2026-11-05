# 营销页面完整功能实现计划

## 目标

补齐 App Store Connect 营销页面控制台的完整闭环：

- 本地创建、详情编辑、复制、删除营销页面。
- 页面文案和截图按语言保存，截图走中心后台上传、预览、删除。
- 同步前展示确认清单，可只同步选中的营销页面内容。
- connector 能接收营销页面同步 payload，mock 和真实模式都不因未知字段失败。
- 同步结果写入历史，页面错误要友好展示。

## 实施步骤

1. 后端领域能力
   - 增加营销页面查询、保存、复制、删除、同步函数。
   - 保存 `StoreMarketingPageLocale.promotional_text` 和 `store_images_json`。
   - 营销页面同步使用独立 operation，记录到 `StoreSyncRun`。

2. Admin 路由和页面
   - Store metadata 页面中的营销页面卡片链接到详情页。
   - 新增营销页面详情页，复用三栏结构：左侧同步项，中间当前项编辑，右侧检查和历史。
   - 支持保存、复制、删除、实时检查、同步确认、图片上传和本地删除。

3. Connector
   - Go connector 的 `SyncRunRequest` 增加 `marketingPage`。
   - mock gateway 校验营销页面 payload。
   - live gateway 先接受并返回明确状态，避免真实 connector 因未知字段失败；后续再补具体 App Store Connect 创建/更新调用。

4. 验证
   - Python 单测覆盖创建、详情保存、多语言、图片上传删除、复制、删除、同步 run。
   - Go 单测覆盖 connector 接收营销页面同步。
   - 运行 ruff、pytest、go test。
   - 合并、推送、部署到远程后验证 API 健康和页面可访问。

## 非目标

- 本次不做 AB 实验。
- 本次不把营销页面绑定到 App 版本。
- 本次不把营销页面创建动作直接同步到商店，只有用户点击同步时才调用 connector。
