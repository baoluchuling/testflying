import 'package:flutter/material.dart';
import 'package:testflying/models/internal_build.dart';
import 'package:testflying/models/workspace_data.dart';

class RemoteWorkspaceDto {
  const RemoteWorkspaceDto({
    required this.apps,
    required this.builds,
    required this.devices,
    required this.developerAccounts,
    required this.notifications,
    required this.installTasks,
    required this.sortOrder,
    required this.profile,
  });

  factory RemoteWorkspaceDto.fromJson(Map<String, Object?> json) {
    final source =
        _map(json['workspace']).isEmpty ? json : _map(json['workspace']);
    return RemoteWorkspaceDto(
      apps: _maps(source['apps']).map(RemoteAppDto.fromJson).toList(),
      builds: _maps(source['builds']).map(RemoteBuildDto.fromJson).toList(),
      devices: _maps(source['devices']).map(RemoteDeviceDto.fromJson).toList(),
      developerAccounts: _maps(source['developerAccounts'])
          .map(RemoteDeveloperAccountDto.fromJson)
          .toList(),
      notifications: _maps(source['notifications'])
          .map(RemoteNotificationDto.fromJson)
          .toList(),
      installTasks: _maps(source['installTasks'])
          .map(RemoteInstallTaskDto.fromJson)
          .toList(),
      sortOrder: RemoteSortOrderDto.fromJson(_map(source['sortOrder'])),
      profile: RemoteProfileDto.fromJson(_map(source['profile'])),
    );
  }

  final List<RemoteAppDto> apps;
  final List<RemoteBuildDto> builds;
  final List<RemoteDeviceDto> devices;
  final List<RemoteDeveloperAccountDto> developerAccounts;
  final List<RemoteNotificationDto> notifications;
  final List<RemoteInstallTaskDto> installTasks;
  final RemoteSortOrderDto sortOrder;
  final RemoteProfileDto profile;

  TestFlightWorkspace toDomain() {
    return TestFlightWorkspace(
      apps: apps.map((app) => app.toDomain()).toList(growable: false),
      builds: builds.map((build) => build.toDomain()).toList(growable: false),
      devices:
          devices.map((device) => device.toDomain()).toList(growable: false),
      developerAccounts: developerAccounts
          .map((account) => account.toDomain())
          .toList(growable: false),
      notifications: notifications
          .map((notification) => notification.toDomain())
          .toList(growable: false),
      installTasks:
          installTasks.map((task) => task.toDomain()).toList(growable: false),
      sortOrder: sortOrder.toDomain(),
      profile: profile.toDomain(),
    );
  }
}

class RemoteAppDto {
  const RemoteAppDto({
    required this.id,
    required this.name,
    required this.addedAt,
    required this.defaultChannel,
    required this.iconKey,
    required this.iconColor,
  });

  factory RemoteAppDto.fromJson(Map<String, Object?> json) {
    return RemoteAppDto(
      id: _string(json['id']),
      name: _string(json['name']),
      addedAt: _string(json['addedAt']),
      defaultChannel: _channel(json['defaultChannel']),
      iconKey: _string(json['iconKey'], fallback: 'app'),
      iconColor: _color(json['iconColor'], const Color(0xFF53606E)),
    );
  }

  final String id;
  final String name;
  final String addedAt;
  final BuildChannel defaultChannel;
  final String iconKey;
  final Color iconColor;

  InternalApp toDomain() {
    return InternalApp(
      id: id,
      name: name,
      addedAt: addedAt,
      defaultChannel: defaultChannel,
      icon: _icon(iconKey, Icons.inventory_2_rounded),
      iconColor: iconColor,
    );
  }
}

class RemoteBuildDto {
  const RemoteBuildDto({
    required this.id,
    required this.name,
    required this.version,
    required this.buildNumber,
    required this.channel,
    required this.environment,
    required this.owner,
    required this.uploadedAt,
    required this.note,
    required this.status,
    required this.iconKey,
    required this.iconColor,
    required this.installInfo,
    required this.progress,
    required this.isPaused,
  });

  factory RemoteBuildDto.fromJson(Map<String, Object?> json) {
    final installInfoJson =
        _map(json['installInfo']).isEmpty ? json : _map(json['installInfo']);
    return RemoteBuildDto(
      id: _string(json['id']),
      name: _string(json['name']),
      version: _string(json['version']),
      buildNumber: _string(json['buildNumber']),
      channel: _channel(json['channel']),
      environment: _string(json['environment']),
      owner: _string(json['owner']),
      uploadedAt: _string(json['uploadedAt']),
      note: _string(json['note']),
      status: _status(json['status']),
      iconKey: _string(json['iconKey'], fallback: 'app'),
      iconColor: _color(json['iconColor'], const Color(0xFF53606E)),
      installInfo: RemoteInstallInfoDto.fromJson(installInfoJson),
      progress: _double(json['progress']),
      isPaused: json['isPaused'] == true,
    );
  }

  final String id;
  final String name;
  final String version;
  final String buildNumber;
  final BuildChannel channel;
  final String environment;
  final String owner;
  final String uploadedAt;
  final String note;
  final BuildStatus status;
  final String iconKey;
  final Color iconColor;
  final RemoteInstallInfoDto installInfo;
  final double? progress;
  final bool isPaused;

  InternalBuild toDomain() {
    return InternalBuild(
      id: id,
      name: name,
      version: version,
      buildNumber: buildNumber,
      channel: channel,
      environment: environment,
      owner: owner,
      uploadedAt: uploadedAt,
      note: note,
      status: status,
      icon: _icon(iconKey, Icons.inventory_2_rounded),
      iconColor: iconColor,
      installInfo: installInfo.toDomain(),
      progress: progress,
      isPaused: isPaused,
    );
  }
}

class RemoteInstallInfoDto {
  const RemoteInstallInfoDto({
    required this.platform,
    required this.installUrl,
    required this.manifestUrl,
    required this.downloadUrl,
    required this.minOsVersion,
    required this.expiresAt,
    required this.isInstallable,
    required this.unavailableReason,
  });

  factory RemoteInstallInfoDto.fromJson(Map<String, Object?> json) {
    return RemoteInstallInfoDto(
      platform: _platform(json['platform']),
      installUrl: _string(json['installUrl']),
      manifestUrl: _nullableString(json['manifestUrl']),
      downloadUrl: _nullableString(json['downloadUrl']),
      minOsVersion: _nullableString(json['minOsVersion']),
      expiresAt: _nullableString(json['expiresAt']),
      isInstallable: json['isInstallable'] != false,
      unavailableReason: _nullableString(json['unavailableReason']),
    );
  }

  final InstallPlatform platform;
  final String installUrl;
  final String? manifestUrl;
  final String? downloadUrl;
  final String? minOsVersion;
  final String? expiresAt;
  final bool isInstallable;
  final String? unavailableReason;

  BuildInstallInfo toDomain() {
    return BuildInstallInfo(
      platform: platform,
      installUrl: installUrl,
      manifestUrl: manifestUrl,
      downloadUrl: downloadUrl,
      minOsVersion: minOsVersion,
      expiresAt: expiresAt,
      isInstallable: isInstallable,
      unavailableReason: unavailableReason,
    );
  }
}

class RemoteInstallTaskDto {
  const RemoteInstallTaskDto({
    required this.buildId,
    required this.deviceId,
    required this.progress,
    required this.isPaused,
  });

  factory RemoteInstallTaskDto.fromJson(Map<String, Object?> json) {
    return RemoteInstallTaskDto(
      buildId: _string(json['buildId']),
      deviceId: _string(json['deviceId']),
      progress: _double(json['progress']) ?? 0,
      isPaused: json['isPaused'] == true,
    );
  }

  final String buildId;
  final String deviceId;
  final double progress;
  final bool isPaused;

  InstallTask toDomain() {
    return InstallTask(
      buildId: buildId,
      deviceId: deviceId,
      progress: progress,
      isPaused: isPaused,
    );
  }
}

class RemoteDeviceDto {
  const RemoteDeviceDto({
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
    required this.isCurrent,
  });

  factory RemoteDeviceDto.fromJson(Map<String, Object?> json) {
    return RemoteDeviceDto(
      id: _string(json['id']),
      name: _string(json['name']),
      owner: _string(json['owner']),
      status: _string(json['status']),
      statusColor: _color(json['statusColor'], const Color(0xFF53606E)),
      detail: _string(json['detail']),
      udid: _string(json['udid']),
      osVersion: _string(json['osVersion']),
      certificateStatus: _string(json['certificateStatus']),
      lastInstalledAt: _string(json['lastInstalledAt']),
      isCurrent: json['isCurrent'] == true,
    );
  }

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

  TestDevice toDomain() {
    return TestDevice(
      id: id,
      name: name,
      owner: owner,
      status: status,
      statusColor: statusColor,
      detail: detail,
      udid: udid,
      osVersion: osVersion,
      certificateStatus: certificateStatus,
      lastInstalledAt: lastInstalledAt,
      isCurrent: isCurrent,
    );
  }
}

class RemoteDeveloperAccountDto {
  const RemoteDeveloperAccountDto({
    required this.id,
    required this.appName,
    required this.teamName,
    required this.remainingDays,
    required this.renewalActionLabel,
  });

  factory RemoteDeveloperAccountDto.fromJson(Map<String, Object?> json) {
    return RemoteDeveloperAccountDto(
      id: _string(json['id']),
      appName: _string(json['appName']),
      teamName: _string(json['teamName']),
      remainingDays: _int(json['remainingDays']),
      renewalActionLabel: _string(json['renewalActionLabel'], fallback: '去续费'),
    );
  }

  final String id;
  final String appName;
  final String teamName;
  final int remainingDays;
  final String renewalActionLabel;

  DeveloperAccount toDomain() {
    return DeveloperAccount(
      id: id,
      appName: appName,
      teamName: teamName,
      remainingDays: remainingDays,
      renewalActionLabel: renewalActionLabel,
    );
  }
}

class RemoteNotificationDto {
  const RemoteNotificationDto({
    required this.type,
    required this.section,
    required this.iconKey,
    required this.title,
    required this.subtitle,
    required this.tag,
    required this.tagColor,
  });

  factory RemoteNotificationDto.fromJson(Map<String, Object?> json) {
    return RemoteNotificationDto(
      type: _noticeType(json['type']),
      section: _string(json['section']),
      iconKey: _string(json['iconKey'], fallback: 'notice'),
      title: _string(json['title']),
      subtitle: _string(json['subtitle']),
      tag: _string(json['tag']),
      tagColor: _color(json['tagColor'], const Color(0xFF2478FF)),
    );
  }

  final NoticeType type;
  final String section;
  final String iconKey;
  final String title;
  final String subtitle;
  final String tag;
  final Color tagColor;

  AppNotification toDomain() {
    return AppNotification(
      type: type,
      section: section,
      icon: _icon(iconKey, type.icon),
      title: title,
      subtitle: subtitle,
      tag: tag,
      tagColor: tagColor,
    );
  }
}

class RemoteSortOrderDto {
  const RemoteSortOrderDto({required this.buildIds});

  factory RemoteSortOrderDto.fromJson(Map<String, Object?> json) {
    return RemoteSortOrderDto(
      buildIds: _strings(json['buildIds']),
    );
  }

  final List<String> buildIds;

  AppSortOrder toDomain() => AppSortOrder(buildIds: buildIds);
}

class RemoteProfileDto {
  const RemoteProfileDto({
    required this.name,
    required this.initial,
    required this.subtitle,
    required this.metrics,
    required this.actions,
    required this.preferences,
  });

  factory RemoteProfileDto.fromJson(Map<String, Object?> json) {
    return RemoteProfileDto(
      name: _string(json['name']),
      initial: _string(json['initial']),
      subtitle: _string(json['subtitle']),
      metrics:
          _maps(json['metrics']).map(RemoteProfileMetricDto.fromJson).toList(),
      actions:
          _maps(json['actions']).map(RemoteProfileActionDto.fromJson).toList(),
      preferences: _maps(json['preferences'])
          .map(RemoteProfilePreferenceDto.fromJson)
          .toList(),
    );
  }

  final String name;
  final String initial;
  final String subtitle;
  final List<RemoteProfileMetricDto> metrics;
  final List<RemoteProfileActionDto> actions;
  final List<RemoteProfilePreferenceDto> preferences;

  UserProfile toDomain() {
    return UserProfile(
      name: name,
      initial: initial,
      subtitle: subtitle,
      metrics:
          metrics.map((metric) => metric.toDomain()).toList(growable: false),
      actions:
          actions.map((action) => action.toDomain()).toList(growable: false),
      preferences: preferences
          .map((preference) => preference.toDomain())
          .toList(growable: false),
    );
  }
}

class RemoteProfileMetricDto {
  const RemoteProfileMetricDto({
    required this.label,
    required this.value,
    required this.iconKey,
  });

  factory RemoteProfileMetricDto.fromJson(Map<String, Object?> json) {
    return RemoteProfileMetricDto(
      label: _string(json['label']),
      value: _string(json['value']),
      iconKey: _string(json['iconKey'], fallback: 'metric'),
    );
  }

  final String label;
  final String value;
  final String iconKey;

  ProfileMetric toDomain() {
    return ProfileMetric(
      label: label,
      value: value,
      icon: _icon(iconKey, Icons.analytics_rounded),
    );
  }
}

class RemoteProfileActionDto {
  const RemoteProfileActionDto({
    required this.iconKey,
    required this.title,
    required this.subtitle,
    required this.message,
  });

  factory RemoteProfileActionDto.fromJson(Map<String, Object?> json) {
    return RemoteProfileActionDto(
      iconKey: _string(json['iconKey'], fallback: 'action'),
      title: _string(json['title']),
      subtitle: _string(json['subtitle']),
      message: _string(json['message']),
    );
  }

  final String iconKey;
  final String title;
  final String subtitle;
  final String message;

  ProfileAction toDomain() {
    return ProfileAction(
      icon: _icon(iconKey, Icons.open_in_new_rounded),
      title: title,
      subtitle: subtitle,
      message: message,
    );
  }
}

class RemoteProfilePreferenceDto {
  const RemoteProfilePreferenceDto({
    required this.title,
    required this.subtitle,
    required this.value,
    required this.message,
  });

  factory RemoteProfilePreferenceDto.fromJson(Map<String, Object?> json) {
    return RemoteProfilePreferenceDto(
      title: _string(json['title']),
      subtitle: _string(json['subtitle']),
      value: json['value'] == true,
      message: _string(json['message']),
    );
  }

  final String title;
  final String subtitle;
  final bool value;
  final String message;

  ProfilePreference toDomain() {
    return ProfilePreference(
      title: title,
      subtitle: subtitle,
      value: value,
      message: message,
    );
  }
}

Map<String, Object?> _map(Object? value) {
  if (value is Map<String, Object?>) {
    return value;
  }
  if (value is Map) {
    return {
      for (final entry in value.entries)
        if (entry.key is String) entry.key as String: entry.value,
    };
  }
  return const {};
}

List<Map<String, Object?>> _maps(Object? value) {
  if (value is! List) {
    return const [];
  }
  return value
      .map(_map)
      .where((item) => item.isNotEmpty)
      .toList(growable: false);
}

List<String> _strings(Object? value) {
  if (value is! List) {
    return const [];
  }
  return value.whereType<String>().toList(growable: false);
}

String _string(Object? value, {String fallback = ''}) {
  return value is String && value.isNotEmpty ? value : fallback;
}

String? _nullableString(Object? value) {
  return value is String && value.isNotEmpty ? value : null;
}

int _int(Object? value) {
  if (value is int) {
    return value;
  }
  if (value is num) {
    return value.round();
  }
  return int.tryParse('$value') ?? 0;
}

double? _double(Object? value) {
  if (value is num) {
    return value.toDouble().clamp(0.0, 1.0);
  }
  return null;
}

Color _color(Object? value, Color fallback) {
  final rawValue = value is String ? value.trim() : '';
  if (rawValue.isEmpty) {
    return fallback;
  }

  final normalized = rawValue
      .replaceFirst('#', '')
      .replaceFirst(RegExp('^0x', caseSensitive: false), '');
  final withAlpha = normalized.length == 6 ? 'FF$normalized' : normalized;
  final parsed = int.tryParse(withAlpha, radix: 16);
  return parsed == null ? fallback : Color(parsed);
}

BuildChannel _channel(Object? value) {
  switch (_normalized(value)) {
    case 'prod':
    case 'production':
    case 'online':
      return BuildChannel.prod;
    case 'dev':
    case 'development':
    default:
      return BuildChannel.dev;
  }
}

BuildStatus _status(Object? value) {
  switch (_normalized(value)) {
    case 'installing':
      return BuildStatus.installing;
    case 'updateavailable':
    case 'update':
      return BuildStatus.updateAvailable;
    case 'installed':
      return BuildStatus.installed;
    case 'expired':
      return BuildStatus.expired;
    case 'available':
    default:
      return BuildStatus.available;
  }
}

InstallPlatform _platform(Object? value) {
  switch (_normalized(value)) {
    case 'android':
      return InstallPlatform.android;
    case 'ios':
    default:
      return InstallPlatform.ios;
  }
}

NoticeType _noticeType(Object? value) {
  switch (_normalized(value)) {
    case 'account':
      return NoticeType.account;
    case 'device':
      return NoticeType.device;
    case 'all':
      return NoticeType.all;
    case 'build':
    default:
      return NoticeType.build;
  }
}

IconData _icon(String key, IconData fallback) {
  switch (_normalized(key)) {
    case 'rocket':
    case 'rocketlaunch':
      return Icons.rocket_launch_rounded;
    case 'layers':
      return Icons.layers_rounded;
    case 'sync':
      return Icons.sync_rounded;
    case 'terminal':
      return Icons.terminal_rounded;
    case 'api':
    case 'viewinar':
      return Icons.view_in_ar_rounded;
    case 'chart':
    case 'showchart':
      return Icons.show_chart_rounded;
    case 'reader':
    case 'inventory':
      return Icons.inventory_2_rounded;
    case 'upload':
      return Icons.cloud_upload_rounded;
    case 'account':
    case 'wallet':
      return Icons.account_balance_wallet_rounded;
    case 'device':
    case 'phone':
      return Icons.phone_iphone_rounded;
    case 'task':
      return Icons.task_alt_rounded;
    case 'bug':
      return Icons.bug_report_rounded;
    case 'history':
      return Icons.history_rounded;
    case 'group':
      return Icons.group_rounded;
    case 'receipt':
      return Icons.receipt_long_rounded;
    case 'download':
    case 'downloaddone':
      return Icons.download_done_rounded;
    case 'pending':
      return Icons.pending_actions_rounded;
    case 'storage':
      return Icons.storage_rounded;
    default:
      return fallback;
  }
}

String _normalized(Object? value) {
  return '$value'.toLowerCase().replaceAll(RegExp(r'[_\-\s]'), '');
}
