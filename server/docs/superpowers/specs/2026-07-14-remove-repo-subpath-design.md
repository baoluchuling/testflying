# 移除仓库子目录设计

## 背景

TestFlying 中一个构建应用始终对应一个独立 Git 仓库，仓库根目录就是应用项目目录，不存在从同一仓库选择子项目的业务场景。现有 `repoSubpath` 配置增加了无效表单项、接口字段、数据库状态和 Runner 路径分支，应当完整移除。

## 目标

- 构建应用只配置 Git 仓库，不再配置仓库子目录。
- Runner 克隆仓库后始终以仓库根目录作为项目目录。
- 管理 API、Runner 任务协议和数据库模型不再暴露或保存 `repoSubpath`。
- 删除针对仓库子目录的校验、错误码和测试。

## 非目标

- 不支持 Monorepo 或仓库内多应用选择。
- 不引入新的工作目录、构建脚本目录或路径映射配置。
- 不修改 Git ref、产物类型、Runner 标签、凭据引用和默认参数的行为。

## 数据模型与迁移

新增 Alembic 迁移，从 `app_build_settings` 删除 `repo_subpath` 列。升级时直接丢弃历史值，因为该数据在新模型中没有业务含义；降级时恢复一个非空、默认空字符串的列，保证迁移可回滚。

ORM 的 `AppBuildSetting` 同步删除 `repo_subpath` 属性。构建任务继续在创建时快照其余共享配置，不再生成路径字段。

## API 与 Runner 协议

管理端保存构建配置的请求和响应删除 `repoSubpath`。构建应用列表、应用详情和 Runner 任务领取响应也不再返回该字段。

Runner 删除 `RepoSubpath` 字段、路径校验和 `repo_subpath_invalid` 失败分类。Git 克隆完成后，`projectDir` 固定指向当前任务的 checkout 根目录，并作为构建输入传给本地构建代理。

这是一次同步升级，不保留旧字段兼容逻辑。中心后台与 Runner 需要使用同一版本部署。

## 后台界面

应用详情和构建工作台中的“仓库子目录”输入、摘要和 TypeScript 字段全部删除。Git 仓库地址直接表示完整应用仓库，不增加替代输入项。

## 错误处理

删除 `invalid_repo_subpath` 管理错误。仓库克隆失败仍按现有 Git checkout 错误处理；仓库根目录不存在属于 checkout 失败，不新增路径类错误。

## 验证

- Alembic 升级、降级、再次升级通过，并验证列确实删除和恢复。
- 保存构建配置、读取应用详情、创建构建任务的 API 测试不再包含 `repoSubpath`。
- Runner 测试验证构建输入中的 `projectDir` 等于 checkout 根目录。
- 应用详情和构建工作台测试验证页面不再展示“仓库子目录”。
- 运行完整 Python、React/TypeScript 和 Go 测试，以及前端生产构建。
