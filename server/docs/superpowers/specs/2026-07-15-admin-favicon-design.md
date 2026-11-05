# 管理后台浏览器图标设计

## 目标

为管理后台增加浏览器标签页图标，避免继续显示浏览器默认空白图标。

## 方案

- 使用 SVG favicon，放在 `admin-web/public/favicon.svg`。
- 图标沿用管理后台左上角品牌标识：深色圆角方块和白色 `TF` 字样。
- 在 `admin-web/index.html` 中通过 `/favicon.svg` 引用。
- 不增加 Web App Manifest、桌面图标或 Apple Touch Icon，保持本次改动聚焦浏览器标签页。

## 验证

- 运行管理后台测试和 TypeScript 检查。
- 运行 Vite 生产构建。
- 确认构建后的 `index.html` 引用了图标，且产物目录包含 `favicon.svg`。
