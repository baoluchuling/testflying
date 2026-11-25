# Independent component releases

中心后台（store-api）、截图节点（screenshot-connector）、商店节点（store-connector）
分别维护各自 Cargo.toml／Cargo.lock 中的版本；不得再使用统一 Connector 版本号。
版本使用规范 major.minor.patch，不允许回退。

只在组件自身生产代码、实际生产依赖或对应安装包发生变化时提高该组件版本。
只改中心不升节点，只改截图节点不升商店节点，只改商店节点不升截图节点。
共享代码变化按实际依赖范围检查，需要升级的是受影响的组件，不是整个工作区。
测试、独立文档、AGENTS.md、无关工作区成员和其他组件版本不触发节点升级。
运行时节点版本目录是发布元数据，更新它不要求中心程序加版本或重新编译。

构建默认按组件生产输入选择 changed；未改组件保留原版本、原程序和原报告来源。
商店节点需要构建时只重打该节点的 Linux/macOS/Windows 包；截图节点仅 macOS arm64；
中心仅构建和部署 API。允许显式重打指定组件，同版本同输入可重打。
不得强制其他组件使用同一版本、重新打包或重新安装。

scripts/component_release.py 负责每组件版本、生产依赖闭包和历史发布输入守卫。
每个正式输出记录 component、version、sourceFingerprint、sourceHashes、sourceCommit、
SHA256 与字节数。旧统一报告必须从它自己的精确 sourceCommit 和原记录恢复，
不得用当前版本冒充旧版，不得删除／改写历史报告来绕过守卫。
中心与节点独立部署都必须验证选定输出；未替换程序的 SHA 和来源保持不变。

后台按节点 kind 展示该类型的最新版本，同时独立展示中心程序版本。
旧节点只上报原 connectorVersion 仍应兼容，不要求修改外部业务调用格式。

所有构建、测试、浏览器和发布继续遵守上级 AGENTS.md 的 artifact-lifecycle 管理要求。
