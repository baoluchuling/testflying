import 'package:flutter/material.dart';
import 'package:testflying/models/internal_build.dart';

enum NoticeType {
  all('全部', Icons.notifications_rounded),
  build('构建', Icons.rocket_launch_rounded),
  account('账号', Icons.account_balance_wallet_rounded),
  device('设备', Icons.devices_rounded);

  const NoticeType(this.label, this.icon);

  final String label;
  final IconData icon;
}

class InternalApp {
  const InternalApp({
    required this.id,
    required this.name,
    required this.addedAt,
    required this.defaultChannel,
    required this.icon,
    required this.iconColor,
  });

  final String id;
  final String name;
  final String addedAt;
  final BuildChannel defaultChannel;
  final IconData icon;
  final Color iconColor;
}

class InstallTask {
  const InstallTask({
    required this.buildId,
    required this.deviceId,
    required this.progress,
    required this.isPaused,
  });

  final String buildId;
  final String deviceId;
  final double progress;
  final bool isPaused;
}

class TestDevice {
  const TestDevice({
    required this.id,
    required this.name,
    required this.owner,
    required this.status,
    required this.statusColor,
    required this.detail,
    required this.udid,
    required this.osVersion,
    required this.certificateStatus,
    required this.lastInstalledAt,
    this.isCurrent = false,
  });

  final String id;
  final String name;
  final String owner;
  final String status;
  final Color statusColor;
  final String detail;
  final String udid;
  final String osVersion;
  final String certificateStatus;
  final String lastInstalledAt;
  final bool isCurrent;
}

class DeveloperAccount {
  const DeveloperAccount({
    required this.id,
    required this.appName,
    required this.teamName,
    required this.remainingDays,
    required this.renewalActionLabel,
  });

  final String id;
  final String appName;
  final String teamName;
  final int remainingDays;
  final String renewalActionLabel;

  String get renewalTitle => '$appName 所属开发者账号 $remainingDays 天后到期';
  String get renewalSubtitle => '$teamName 需要续费后继续签名内部测试包';
  String get certificateSubtitle => '有效期剩余 $remainingDays 天';
}

class AppNotification {
  const AppNotification({
    required this.type,
    required this.section,
    required this.icon,
    required this.title,
    required this.subtitle,
    required this.tag,
    required this.tagColor,
  });

  final NoticeType type;
  final String section;
  final IconData icon;
  final String title;
  final String subtitle;
  final String tag;
  final Color tagColor;
}

class ProfileMetric {
  const ProfileMetric({
    required this.label,
    required this.value,
    required this.icon,
  });

  final String label;
  final String value;
  final IconData icon;
}

class ProfileAction {
  const ProfileAction({
    required this.icon,
    required this.title,
    required this.subtitle,
    required this.message,
  });

  final IconData icon;
  final String title;
  final String subtitle;
  final String message;
}

class ProfilePreference {
  const ProfilePreference({
    required this.title,
    required this.subtitle,
    required this.value,
    required this.message,
  });

  final String title;
  final String subtitle;
  final bool value;
  final String message;
}

class UserProfile {
  const UserProfile({
    required this.name,
    required this.initial,
    required this.subtitle,
    required this.metrics,
    required this.actions,
    required this.preferences,
  });

  final String name;
  final String initial;
  final String subtitle;
  final List<ProfileMetric> metrics;
  final List<ProfileAction> actions;
  final List<ProfilePreference> preferences;
}

class AppSortOrder {
  const AppSortOrder({required this.buildIds});

  final List<String> buildIds;
}

class TestFlightWorkspace {
  const TestFlightWorkspace({
    required this.apps,
    required this.builds,
    required this.devices,
    required this.developerAccounts,
    required this.notifications,
    required this.installTasks,
    required this.sortOrder,
    required this.profile,
  });

  final List<InternalApp> apps;
  final List<InternalBuild> builds;
  final List<TestDevice> devices;
  final List<DeveloperAccount> developerAccounts;
  final List<AppNotification> notifications;
  final List<InstallTask> installTasks;
  final AppSortOrder sortOrder;
  final UserProfile profile;

  TestDevice get currentDevice =>
      devices.firstWhere((device) => device.isCurrent);

  DeveloperAccount? get renewalAccount =>
      developerAccounts.isEmpty ? null : developerAccounts.first;

  TestFlightWorkspace copyWith({
    List<InternalApp>? apps,
    List<InternalBuild>? builds,
    List<TestDevice>? devices,
    List<DeveloperAccount>? developerAccounts,
    List<AppNotification>? notifications,
    List<InstallTask>? installTasks,
    AppSortOrder? sortOrder,
    UserProfile? profile,
  }) {
    return TestFlightWorkspace(
      apps: apps ?? this.apps,
      builds: builds ?? this.builds,
      devices: devices ?? this.devices,
      developerAccounts: developerAccounts ?? this.developerAccounts,
      notifications: notifications ?? this.notifications,
      installTasks: installTasks ?? this.installTasks,
      sortOrder: sortOrder ?? this.sortOrder,
      profile: profile ?? this.profile,
    );
  }
}
