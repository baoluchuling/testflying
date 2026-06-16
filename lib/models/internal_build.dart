import 'package:flutter/material.dart';

enum BuildChannel {
  dev('开发环境', Color(0xFF2478FF), Color(0xFFE8F1FF)),
  prod('线上环境', Color(0xFF20864A), Color(0xFFE7F8EE));

  const BuildChannel(this.label, this.foreground, this.background);

  final String label;
  final Color foreground;
  final Color background;
}

enum BuildStatus {
  installing,
  updateAvailable,
  available,
  installed,
  expired,
}

enum BuildFilter {
  all('全部'),
  dev('开发环境'),
  prod('线上环境'),
  installed('已安装');

  const BuildFilter(this.label);

  final String label;
}

enum InstallPlatform {
  ios('iOS'),
  android('Android');

  const InstallPlatform(this.label);

  final String label;
}

class BuildInstallInfo {
  const BuildInstallInfo({
    required this.platform,
    required this.installUrl,
    this.manifestUrl,
    this.downloadUrl,
    this.minOsVersion,
    this.expiresAt,
    this.isInstallable = true,
    this.unavailableReason,
  });

  final InstallPlatform platform;
  final String installUrl;
  final String? manifestUrl;
  final String? downloadUrl;
  final String? minOsVersion;
  final String? expiresAt;
  final bool isInstallable;
  final String? unavailableReason;

  bool get usesItmsServices =>
      platform == InstallPlatform.ios &&
      installUrl.startsWith('itms-services://');
}

class InternalBuild {
  const InternalBuild({
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
    required this.icon,
    required this.iconColor,
    required this.installInfo,
    this.progress,
    this.isPaused = false,
  });

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
  final IconData icon;
  final Color iconColor;
  final BuildInstallInfo installInfo;
  final double? progress;
  final bool isPaused;

  String get actionLabel {
    switch (status) {
      case BuildStatus.installing:
        return isPaused ? '继续' : '暂停';
      case BuildStatus.updateAvailable:
        return '更新';
      case BuildStatus.available:
        return '安装';
      case BuildStatus.installed:
        return '重装';
      case BuildStatus.expired:
        return '过期';
    }
  }

  String get statusLabel {
    switch (status) {
      case BuildStatus.installing:
        return isPaused ? '已暂停' : '安装中';
      case BuildStatus.updateAvailable:
        return '待更新';
      case BuildStatus.available:
        return '可安装';
      case BuildStatus.installed:
        return '已安装';
      case BuildStatus.expired:
        return '已过期';
    }
  }

  InternalBuild copyWith({
    BuildStatus? status,
    double? progress,
    bool? isPaused,
    BuildInstallInfo? installInfo,
  }) {
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
      status: status ?? this.status,
      icon: icon,
      iconColor: iconColor,
      installInfo: installInfo ?? this.installInfo,
      progress: progress ?? this.progress,
      isPaused: isPaused ?? this.isPaused,
    );
  }
}
