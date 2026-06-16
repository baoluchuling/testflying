# testflying 设计 QA

final result: passed

## 验证对象

- 目标方向：内部 iOS 内测分发工具首页。
- 参考稿：`/Users/admin/.codex/generated_images/019ec932-7686-7420-af58-27ef95b6c399/ig_029879ecda96a9d8016a2f7eac9d08819aa36d0f407407c018.png`
- iOS 首屏截图：`/Users/admin/ai_project/apps/testflying/design-qa-assets/ios-flutter-run.png`
- iOS 交互截图：
  - QA 筛选：`/Users/admin/ai_project/apps/testflying/design-qa-assets/ios-after-qa-click.png`
  - 暂停安装：`/Users/admin/ai_project/apps/testflying/design-qa-assets/ios-after-pause-real.png`
  - 构建详情：`/Users/admin/ai_project/apps/testflying/design-qa-assets/ios-after-detail-click.png`
  - 详情页动作：`/Users/admin/ai_project/apps/testflying/design-qa-assets/ios-after-detail-action.png`

## 已检查项

- 首页在 iPhone 17 Pro 模拟器中可安装、可启动、可渲染。
- 首屏包含内部测试工具所需的信息层级：设备登记、系统版本、证书权限、今日构建、渠道筛选、安装中、可更新、底部导航。
- 列表项包含内部测试字段：版本号、build number、QA/Beta/Internal 渠道、Test/Prod 环境、负责人、上传时间、安装进度和操作按钮。
- 移动端文字没有明显重叠；汇总卡已从四列调整为 2x2，避免 `iPhone 15 Pro`、`iOS 17.5.1`、`证书与权限` 被截断。
- 主要交互通过 widget test 覆盖：页面结构渲染、QA 渠道筛选、刷新/暂停/底部导航点击、构建详情进入。
- 在 Simulator 中通过辅助功能控制实际点击确认：底部导航可切换、QA 筛选可切换、安装暂停按钮可切换为继续、卡片箭头可进入详情页、详情页动作按钮可触发反馈。

## 验证命令

```bash
/Users/admin/fvm/versions/3.29.2/bin/cache/dart-sdk/bin/dart /Users/admin/fvm/versions/3.29.2/bin/cache/flutter_tools.snapshot test
/Users/admin/fvm/versions/3.29.2/bin/cache/dart-sdk/bin/dart /Users/admin/fvm/versions/3.29.2/bin/cache/flutter_tools.snapshot analyze
/Users/admin/fvm/versions/3.29.2/bin/cache/dart-sdk/bin/dart /Users/admin/fvm/versions/3.29.2/bin/cache/flutter_tools.snapshot build ios --simulator --debug
/Users/admin/fvm/versions/3.29.2/bin/cache/dart-sdk/bin/dart /Users/admin/fvm/versions/3.29.2/bin/cache/flutter_tools.snapshot run -d 99DC80A3-9737-47E6-8C21-0F560E29A4BF
xcrun simctl io 99DC80A3-9737-47E6-8C21-0F560E29A4BF screenshot /Users/admin/ai_project/apps/testflying/design-qa-assets/ios-flutter-run.png
```

## 备注

- 直接使用 `flutter` wrapper 会被本机 FVM wrapper 卡在 engine version 更新；本次验证使用 `flutter_tools.snapshot` 运行 Flutter 命令。
- 首次尝试安装到模拟器时拿到了旧 build 缓存，已通过 `flutter clean` 后重新构建解决。
- 对 debug simulator 包直接执行 `simctl install` + `simctl launch` 会出现白屏；使用 `flutter run` 启动并 attach 后可正常渲染和交互。detach 后应用仍留在模拟器中。
- 自动化滚动受当前 macOS/Simulator 脚本接口限制未能稳定发送；本次重点验证的是点击交互。Flutter widget test 已覆盖列表内容和交互状态变化。
