# Connector 交付与失败协议

仅在截图 Connector 任务中使用本协议；普通本地采集仍按主工作流交付原图。

先调用 `get_capture_task({})`，核对平台、包名、原始提示词、目标顺序及 `requiresFinalization`。每个 target 是独立目标，不能按设备、语言和页面另行展开组合。原始提示词规定画面要求；任务身份和目标索引以工具返回为准。

## 候选图与重拍

- 使用当前节点提供的 `validate_simulator`、`capture_screenshot`、`upload_screenshot`，保留实际截图 ID 与零基 `targetIndex`。Skill 自带 runner 或助手可用于准备、导航和检查；Connector 任务的正式交付仍走 Connector 工具。
- `get_capture_task` 中的 `pendingScreenshot` 是当前本地图；`uploadedScreenshots` 是已确认上传的候选图。原图审阅不合格时，如节点提供 `discard_screenshot`，按对应 ID 和索引丢弃，再调整页面、重新采集和检查；已确认上传的候选图也可在最终提交前撤销。
- 上传已经开始或结果未知时，先用同一个 screenshotId 重试 `upload_screenshot` 确认结果。撤销结果未知时先重试同一次撤销；最终提交结果未知时先重试最终提交。不能换 ID 重放未知上传，也不能把未知结果当作失败。
- `requiresFinalization=true`：所有图上传后仍只是候选图。逐张审阅、补拍合格后调用 `finish_capture_task({})`；只有其 `delivered=true` 才完成整单。最终提交开始后不再改图。失败整单不会把未最终提交的候选图回传外部。
- `requiresFinalization=false`：沿用节点的旧交付协议，以 `upload_screenshot` 返回 `delivered=true` 为完成；不要调用节点未提供的工具，也不要尝试撤回已经完成回传的历史任务。

## 阻塞与失败原因

可恢复的工具错误在当前 Goal 内继续修复或等待重试。没有现成模拟器时自行准备或创建；不能仅因找不到现成设备就失败。实际尝试后，仍因开发环境、运行时、必要凭据或其他明确阻塞无法完成时：

1. 如果提供 `report_capture_failure`，调用 `report_capture_failure({"reason":"实际阻塞事实、已尝试的处理及仍缺少的条件"})`。原因写简洁、具体的自然语言，区分已确认事实和不确定信息。该工具是确定终止任务；不要为可恢复错误调用它。
2. 原因不得包含完整提示词、密钥、Token、Cookie、签名 URL、私有目录或大段日志。不能把最后看到的错误猜成根因。上传、撤销或最终提交还未确认时，先解决该操作；节点会拒绝用失败上报覆盖未知交付。
3. 旧节点未提供该工具时，在当前会话明确说明实际阻塞，再按已有工具结果结束；不要虚构工具或声称中心已经收到详细原因。

不要在未获得最终交付确认时主动把 Goal 标成 complete。中心的新查询和失败回调以 `errorCode` 保存稳定错误码，以 `error` 保存脱敏说明；未记录到原生原因时会明确告知，不能承诺还原历史根因。
