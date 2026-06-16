import 'package:flutter/material.dart';
import 'package:testflying/models/internal_build.dart';
import 'package:testflying/models/workspace_data.dart';
import 'package:testflying/services/install_launcher.dart';
import 'package:testflying/services/testflight_service.dart';
import 'package:testflying/services/workspace_preferences_store.dart';

class MockTestFlightService implements TestFlightService {
  MockTestFlightService({
    WorkspacePreferencesStore preferencesStore =
        const WorkspacePreferencesStore(),
    InstallLauncher installLauncher = const UrlInstallLauncher(),
  })  : _preferencesStore = preferencesStore,
        _installLauncher = installLauncher;

  final WorkspacePreferencesStore _preferencesStore;
  final InstallLauncher _installLauncher;

  @override
  Future<TestFlightWorkspace> loadWorkspace() async {
    final preferences = await _preferencesStore.load();
    return preferences.applyTo(seedWorkspace());
  }

  static TestFlightWorkspace seedWorkspace() {
    const builds = _seedBuilds;
    return TestFlightWorkspace(
      apps: _seedApps,
      builds: builds,
      devices: _seedDevices,
      developerAccounts: _seedDeveloperAccounts,
      notifications: _seedNotifications,
      installTasks: [
        const InstallTask(
          buildId: 'aurora',
          deviceId: 'current-iphone',
          progress: .35,
          isPaused: false,
        ),
      ],
      sortOrder: AppSortOrder(
        buildIds: builds.map((build) => build.id).toList(),
      ),
      profile: _seedProfile,
    );
  }

  @override
  Future<TestFlightWorkspace> toggleInstallState(
    TestFlightWorkspace workspace,
    InternalBuild selectedBuild,
  ) async {
    if (selectedBuild.status != BuildStatus.installing) {
      final result = await _installLauncher.launch(selectedBuild);
      if (!result.success) {
        throw InstallLaunchException(result.message);
      }
    }

    final builds = workspace.builds.map((build) {
      if (build.id != selectedBuild.id) {
        return build;
      }
      if (build.status == BuildStatus.installing) {
        return build.copyWith(isPaused: !build.isPaused);
      }
      return build.copyWith(
        status: BuildStatus.installing,
        progress: .08,
        isPaused: false,
      );
    }).toList();

    final toggledBuild =
        builds.firstWhere((build) => build.id == selectedBuild.id);
    final installTasks = [
      for (final task in workspace.installTasks)
        if (task.buildId != selectedBuild.id) task,
      if (toggledBuild.status == BuildStatus.installing)
        InstallTask(
          buildId: toggledBuild.id,
          deviceId: workspace.currentDevice.id,
          progress: toggledBuild.progress ?? .08,
          isPaused: toggledBuild.isPaused,
        ),
    ];

    final updatedWorkspace = workspace.copyWith(
      builds: builds,
      installTasks: installTasks,
    );
    await _preferencesStore.save(updatedWorkspace);
    return updatedWorkspace;
  }

  @override
  Future<TestFlightWorkspace> reorderVisibleBuilds(
    TestFlightWorkspace workspace,
    List<InternalBuild> visibleBuilds,
    int oldIndex,
    int newIndex,
  ) async {
    if (oldIndex < 0 || oldIndex >= visibleBuilds.length) {
      return workspace;
    }
    var targetIndex = newIndex;
    if (targetIndex > oldIndex) {
      targetIndex -= 1;
    }
    if (targetIndex < 0 ||
        targetIndex >= visibleBuilds.length ||
        oldIndex == targetIndex) {
      return workspace;
    }

    final reorderedVisible = [...visibleBuilds];
    final movingBuild = reorderedVisible.removeAt(oldIndex);
    reorderedVisible.insert(targetIndex, movingBuild);
    final visibleIds = visibleBuilds.map((build) => build.id).toSet();
    var visibleIndex = 0;

    final builds = workspace.builds.map((build) {
      if (!visibleIds.contains(build.id)) {
        return build;
      }
      return reorderedVisible[visibleIndex++];
    }).toList();

    final updatedWorkspace = workspace.copyWith(
      builds: builds,
      sortOrder:
          AppSortOrder(buildIds: builds.map((build) => build.id).toList()),
    );
    await _preferencesStore.save(updatedWorkspace);
    return updatedWorkspace;
  }

  static const _seedApps = [
    InternalApp(
      id: 'aurora',
      name: 'Aurora Mobile',
      addedAt: '今天 11:08',
      defaultChannel: BuildChannel.dev,
      icon: Icons.rocket_launch_rounded,
      iconColor: Color(0xFF243D78),
    ),
    InternalApp(
      id: 'dataflow',
      name: 'DataFlow',
      addedAt: '今天 10:24',
      defaultChannel: BuildChannel.dev,
      icon: Icons.layers_rounded,
      iconColor: Color(0xFF1EC58A),
    ),
    InternalApp(
      id: 'insight',
      name: 'Insight Pro',
      addedAt: '今天 09:15',
      defaultChannel: BuildChannel.prod,
      icon: Icons.sync_rounded,
      iconColor: Color(0xFF7E3FF2),
    ),
    InternalApp(
      id: 'ops',
      name: 'Ops Console',
      addedAt: '昨天 18:40',
      defaultChannel: BuildChannel.dev,
      icon: Icons.terminal_rounded,
      iconColor: Color(0xFF17191F),
    ),
    InternalApp(
      id: 'api',
      name: 'API Explorer',
      addedAt: '昨天 16:22',
      defaultChannel: BuildChannel.dev,
      icon: Icons.view_in_ar_rounded,
      iconColor: Color(0xFF3FA2F6),
    ),
    InternalApp(
      id: 'metrics',
      name: 'Metrics Hub',
      addedAt: '昨天 14:05',
      defaultChannel: BuildChannel.prod,
      icon: Icons.show_chart_rounded,
      iconColor: Color(0xFFFF7A1A),
    ),
    InternalApp(
      id: 'reader',
      name: 'ReaderKit Demo',
      addedAt: '周五 17:20',
      defaultChannel: BuildChannel.dev,
      icon: Icons.inventory_2_rounded,
      iconColor: Color(0xFF53606E),
    ),
  ];

  static const _seedBuilds = [
    InternalBuild(
      id: 'aurora',
      name: 'Aurora Mobile',
      version: '2.3.0',
      buildNumber: '23045',
      channel: BuildChannel.dev,
      environment: '开发环境',
      owner: '张三',
      uploadedAt: '今天 11:08',
      note: '登录链路开发环境',
      status: BuildStatus.installing,
      icon: Icons.rocket_launch_rounded,
      iconColor: Color(0xFF243D78),
      installInfo: BuildInstallInfo(
        platform: InstallPlatform.ios,
        installUrl:
            'itms-services://?action=download-manifest&url=https%3A%2F%2Fdist.internal.example.com%2Fmanifest%2Faurora-mobile-23045.plist',
        manifestUrl:
            'https://dist.internal.example.com/manifest/aurora-mobile-23045.plist',
        downloadUrl:
            'https://dist.internal.example.com/ipa/aurora-mobile-23045.ipa',
        minOsVersion: 'iOS 16.0',
        expiresAt: '5 天后',
      ),
      progress: .35,
    ),
    InternalBuild(
      id: 'dataflow',
      name: 'DataFlow',
      version: '1.8.4',
      buildNumber: '18423',
      channel: BuildChannel.dev,
      environment: '开发环境',
      owner: '王琳',
      uploadedAt: '今天 10:24',
      note: '修复数据同步异常问题',
      status: BuildStatus.updateAvailable,
      icon: Icons.layers_rounded,
      iconColor: Color(0xFF1EC58A),
      installInfo: BuildInstallInfo(
        platform: InstallPlatform.ios,
        installUrl:
            'itms-services://?action=download-manifest&url=https%3A%2F%2Fdist.internal.example.com%2Fmanifest%2Fdataflow-18423.plist',
        manifestUrl:
            'https://dist.internal.example.com/manifest/dataflow-18423.plist',
        downloadUrl: 'https://dist.internal.example.com/ipa/dataflow-18423.ipa',
        minOsVersion: 'iOS 16.0',
        expiresAt: '5 天后',
      ),
    ),
    InternalBuild(
      id: 'insight',
      name: 'Insight Pro',
      version: '5.1.0',
      buildNumber: '51002',
      channel: BuildChannel.prod,
      environment: '线上环境',
      owner: '刘洋',
      uploadedAt: '今天 09:15',
      note: '优化报表渲染性能',
      status: BuildStatus.updateAvailable,
      icon: Icons.sync_rounded,
      iconColor: Color(0xFF7E3FF2),
      installInfo: BuildInstallInfo(
        platform: InstallPlatform.ios,
        installUrl:
            'itms-services://?action=download-manifest&url=https%3A%2F%2Fdist.internal.example.com%2Fmanifest%2Finsight-pro-51002.plist',
        manifestUrl:
            'https://dist.internal.example.com/manifest/insight-pro-51002.plist',
        downloadUrl:
            'https://dist.internal.example.com/ipa/insight-pro-51002.ipa',
        minOsVersion: 'iOS 16.0',
        expiresAt: '5 天后',
      ),
    ),
    InternalBuild(
      id: 'ops',
      name: 'Ops Console',
      version: '1.2.0',
      buildNumber: '12001',
      channel: BuildChannel.dev,
      environment: '开发环境',
      owner: '张三',
      uploadedAt: '昨天 18:40',
      note: '初始化版本发布',
      status: BuildStatus.available,
      icon: Icons.terminal_rounded,
      iconColor: Color(0xFF17191F),
      installInfo: BuildInstallInfo(
        platform: InstallPlatform.android,
        installUrl:
            'https://dist.internal.example.com/apk/ops-console-12001.apk',
        downloadUrl:
            'https://dist.internal.example.com/apk/ops-console-12001.apk',
        minOsVersion: 'Android 10',
        expiresAt: '12 天后',
      ),
    ),
    InternalBuild(
      id: 'api',
      name: 'API Explorer',
      version: '3.0.0',
      buildNumber: '30087',
      channel: BuildChannel.dev,
      environment: '开发环境',
      owner: '李四',
      uploadedAt: '昨天 16:22',
      note: '新增接口调试历史',
      status: BuildStatus.available,
      icon: Icons.view_in_ar_rounded,
      iconColor: Color(0xFF3FA2F6),
      installInfo: BuildInstallInfo(
        platform: InstallPlatform.android,
        installUrl:
            'https://dist.internal.example.com/apk/api-explorer-30087.apk',
        downloadUrl:
            'https://dist.internal.example.com/apk/api-explorer-30087.apk',
        minOsVersion: 'Android 10',
        expiresAt: '12 天后',
      ),
    ),
    InternalBuild(
      id: 'metrics',
      name: 'Metrics Hub',
      version: '2.7.1',
      buildNumber: '27031',
      channel: BuildChannel.prod,
      environment: '线上环境',
      owner: '王五',
      uploadedAt: '昨天 14:05',
      note: '支持自定义监控告警',
      status: BuildStatus.available,
      icon: Icons.show_chart_rounded,
      iconColor: Color(0xFFFF7A1A),
      installInfo: BuildInstallInfo(
        platform: InstallPlatform.ios,
        installUrl:
            'itms-services://?action=download-manifest&url=https%3A%2F%2Fdist.internal.example.com%2Fmanifest%2Fmetrics-hub-27031.plist',
        manifestUrl:
            'https://dist.internal.example.com/manifest/metrics-hub-27031.plist',
        downloadUrl:
            'https://dist.internal.example.com/ipa/metrics-hub-27031.ipa',
        minOsVersion: 'iOS 16.0',
        expiresAt: '5 天后',
      ),
    ),
    InternalBuild(
      id: 'reader',
      name: 'ReaderKit Demo',
      version: '0.9.8',
      buildNumber: '9801',
      channel: BuildChannel.dev,
      environment: '开发环境',
      owner: '陈晨',
      uploadedAt: '周五 17:20',
      note: '验证分页排版回归',
      status: BuildStatus.installed,
      icon: Icons.inventory_2_rounded,
      iconColor: Color(0xFF53606E),
      installInfo: BuildInstallInfo(
        platform: InstallPlatform.ios,
        installUrl:
            'itms-services://?action=download-manifest&url=https%3A%2F%2Fdist.internal.example.com%2Fmanifest%2Freaderkit-demo-9801.plist',
        manifestUrl:
            'https://dist.internal.example.com/manifest/readerkit-demo-9801.plist',
        downloadUrl:
            'https://dist.internal.example.com/ipa/readerkit-demo-9801.ipa',
        minOsVersion: 'iOS 16.0',
        expiresAt: '5 天后',
      ),
    ),
  ];

  static const _seedDevices = [
    TestDevice(
      id: 'current-iphone',
      name: 'iPhone 15 Pro',
      owner: '当前设备',
      status: '已登记',
      statusColor: Color(0xFF22C55E),
      detail: 'iOS 17.5.1 · 开发环境',
      udid: '00008130-001E5D223A11801E',
      osVersion: 'iOS 17.5.1',
      certificateStatus: '企业签名有效',
      lastInstalledAt: '今天 11:08',
      isCurrent: true,
    ),
    TestDevice(
      id: 'dev-iphone-15',
      name: '开发-iPhone-15',
      owner: '张三',
      status: '在线',
      statusColor: Color(0xFF22C55E),
      detail: 'iOS 17.5.1 · 开发环境',
      udid: '00008130-001E5D223A11801E',
      osVersion: 'iOS 17.5.1',
      certificateStatus: '企业签名有效',
      lastInstalledAt: '今天 11:08',
    ),
    TestDevice(
      id: 'prod-ipad-air',
      name: '线上-iPad-Air',
      owner: '刘洋',
      status: '待确认',
      statusColor: Color(0xFFF59E0B),
      detail: 'iPadOS 17.4 · 线上环境',
      udid: '00008110-001A58A11223011E',
      osVersion: 'iPadOS 17.4',
      certificateStatus: '企业签名有效',
      lastInstalledAt: '昨天 18:12',
    ),
    TestDevice(
      id: 'ops-iphone-14',
      name: 'Ops-iPhone-14',
      owner: '王琳',
      status: '证书将过期',
      statusColor: Color(0xFFDC2626),
      detail: 'iOS 16.7 · 开发环境',
      udid: '00008120-001C45B33011801E',
      osVersion: 'iOS 16.7',
      certificateStatus: '企业签名将过期',
      lastInstalledAt: '昨天 16:22',
    ),
  ];

  static const _seedDeveloperAccounts = [
    DeveloperAccount(
      id: 'apple-team-a',
      appName: 'Aurora Mobile',
      teamName: 'Apple Developer Team A',
      remainingDays: 5,
      renewalActionLabel: '去续费',
    ),
  ];

  static const _seedNotifications = [
    AppNotification(
      type: NoticeType.build,
      section: '今天',
      icon: Icons.cloud_upload_rounded,
      title: 'DataFlow 新构建已上传',
      subtitle: '1.8.4 (18423) · 王琳 · 今天 10:24',
      tag: '待更新',
      tagColor: Color(0xFF2478FF),
    ),
    AppNotification(
      type: NoticeType.account,
      section: '今天',
      icon: Icons.account_balance_wallet_rounded,
      title: '开发者账号即将到期',
      subtitle: 'Aurora Mobile · Apple Developer Team A · 剩余 5 天',
      tag: '续费',
      tagColor: Color(0xFFDC2626),
    ),
    AppNotification(
      type: NoticeType.build,
      section: '今天',
      icon: Icons.task_alt_rounded,
      title: 'Aurora Mobile 安装已暂停',
      subtitle: '开发环境 · iPhone 15 Pro',
      tag: '安装任务',
      tagColor: Color(0xFF7B61FF),
    ),
    AppNotification(
      type: NoticeType.device,
      section: '昨天',
      icon: Icons.phone_iphone_rounded,
      title: 'Ops-iPhone-14 已加入设备池',
      subtitle: '开发环境 · 张三审批',
      tag: '设备',
      tagColor: Color(0xFF20864A),
    ),
    AppNotification(
      type: NoticeType.build,
      section: '昨天',
      icon: Icons.bug_report_rounded,
      title: 'API Explorer 崩溃日志已收集',
      subtitle: '3.0.0 (30087) · 2 条新日志',
      tag: '日志',
      tagColor: Color(0xFFB76613),
    ),
  ];

  static const _seedProfile = UserProfile(
    name: '张三',
    initial: '张',
    subtitle: '测试管理员 · 移动端测试组',
    metrics: [
      ProfileMetric(
        label: '今日安装',
        value: '12',
        icon: Icons.download_done_rounded,
      ),
      ProfileMetric(
        label: '负责应用',
        value: '4',
        icon: Icons.inventory_2_rounded,
      ),
      ProfileMetric(
        label: '待处理',
        value: '3',
        icon: Icons.pending_actions_rounded,
      ),
      ProfileMetric(
        label: '可用配额',
        value: '86%',
        icon: Icons.storage_rounded,
      ),
    ],
    actions: [
      ProfileAction(
        icon: Icons.history_rounded,
        title: '安装历史',
        subtitle: '查看最近 30 天安装记录',
        message: '安装历史已打开',
      ),
      ProfileAction(
        icon: Icons.group_rounded,
        title: '团队成员',
        subtitle: '12 人 · 3 个角色',
        message: '团队成员已打开',
      ),
      ProfileAction(
        icon: Icons.receipt_long_rounded,
        title: '审计日志',
        subtitle: '构建上传、签名、安装记录',
        message: '审计日志已打开',
      ),
    ],
    preferences: [
      ProfilePreference(
        title: '只看我的应用',
        subtitle: '首页优先展示我负责的构建',
        value: true,
        message: '偏好已保存',
      ),
      ProfilePreference(
        title: '安装完成通知',
        subtitle: '构建安装结束后推送通知',
        value: true,
        message: '通知偏好已保存',
      ),
    ],
  );
}
