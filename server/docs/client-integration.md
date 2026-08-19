# 客户端集成边界

客户端拼装 UI 时使用这个公式：

```text
服务端 catalog/workspace
+ 客户端本地安装状态
+ 客户端本地暂停/下载进度
+ 客户端本地排序
+ 客户端本地通知已读
= UI workspace
```

## 服务端返回什么

服务端返回稳定分发事实：

- 应用列表。
- 构建列表。
- 安装入口 URL。
- 当前设备可见的构建。
- 设备登记事实。
- 开发者账号续费事实。
- 服务端通知 feed。

`workspace` 是首屏聚合接口，客户端可以直接把 `apps`、`builds`、`devices`、`developerAccounts`、`notifications`、`profile` 映射到页面数据。

## 客户端自己保存什么

客户端继续保存这些本地状态：

- 某个构建是否已安装。
- 某个构建是否安装中。
- 下载进度。
- 暂停/继续状态。
- 应用列表手动排序。
- 通知是否已读。
- 当前 tab、筛选条件、sheet 展开状态、滚动位置。

服务端不会返回这些状态字段：`isRead`、`readAt`、`installedAt`、`installState`、`progress`。

## 安装动作

客户端点击安装时只打开 `build.installInfo.installUrl`：

- iOS：打开 `itms-services://?action=download-manifest&url=<manifest.plist>`。
- Android：打开 APK 下载地址或系统下载入口。

安装成功、失败、暂停和进度都由设备端自己处理。服务端不会创建 install task，也不会知道某台设备是否真正安装成功。

## 新应用如何出现

新应用不由设备端手动添加。推荐流程：

1. CI、后台或管理脚本上传 IPA/APK 到 `POST /v1/test-distribution/uploads`。
2. 服务端解析包信息并创建应用、构建、制品和通知。
3. 服务端把构建授权给同平台已登记设备。
4. 客户端刷新 `workspace` 后看到新应用或新构建。

这样应用目录来源一致，设备端不需要维护“新增应用”入口。

## 远端客户端需要调整的点

远端客户端不要再调用这些旧接口：

- install-task 创建/更新/读取接口。
- 用户排序写接口。
- 通知已读写接口。

对应行为应改为本地状态更新，然后按需重新拉取 `workspace` 校准服务端事实。
