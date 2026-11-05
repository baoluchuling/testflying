import 'package:flutter/foundation.dart';
import 'package:testflying/models/internal_build.dart';
import 'package:url_launcher/url_launcher.dart';

typedef UrlOpener = Future<bool> Function(Uri uri, LaunchMode mode);

abstract class InstallLauncher {
  Future<InstallLaunchResult> launch(InternalBuild build);
}

class InstallLaunchResult {
  const InstallLaunchResult._({
    required this.success,
    required this.message,
  });

  const InstallLaunchResult.success(String message)
      : this._(success: true, message: message);

  const InstallLaunchResult.failure(String message)
      : this._(success: false, message: message);

  final bool success;
  final String message;
}

class InstallLaunchException implements Exception {
  const InstallLaunchException(this.message);

  final String message;

  @override
  String toString() => message;
}

class UrlInstallLauncher implements InstallLauncher {
  const UrlInstallLauncher()
      : _openUrlOverride = null,
        _isWebOverride = null;

  UrlInstallLauncher.forTesting({
    required UrlOpener openUrl,
    required bool isWeb,
  })  : _openUrlOverride = openUrl,
        _isWebOverride = isWeb;

  final UrlOpener? _openUrlOverride;
  final bool? _isWebOverride;

  bool get _isWeb => _isWebOverride ?? kIsWeb;

  @override
  Future<InstallLaunchResult> launch(InternalBuild build) async {
    final installInfo = build.installInfo;
    if (!installInfo.isInstallable) {
      return InstallLaunchResult.failure(
        installInfo.unavailableReason ?? '${build.name} 当前不可安装',
      );
    }

    final candidates = _launchCandidates(installInfo);
    if (candidates.isEmpty) {
      return InstallLaunchResult.failure('${build.name} 安装链接无效');
    }

    for (final candidate in candidates) {
      if (await _tryLaunch(candidate)) {
        return InstallLaunchResult.success(
          _successMessage(build, candidate),
        );
      }
    }

    return InstallLaunchResult.failure(
      '${installInfo.platform.label} 安装入口打开失败',
    );
  }

  List<Uri> _launchCandidates(BuildInstallInfo installInfo) {
    final candidates = <Uri?>[
      _parseUri(installInfo.installUrl),
    ];
    if (_isWeb) {
      switch (installInfo.platform) {
        case InstallPlatform.ios:
          if (installInfo.usesItmsServices) {
            candidates.add(_parseUri(installInfo.manifestUrl));
            candidates.add(_parseUri(installInfo.downloadUrl));
          }
          break;
        case InstallPlatform.android:
          candidates.add(_parseUri(installInfo.downloadUrl));
          break;
      }
    }
    return _dedupe(candidates);
  }

  Future<bool> _tryLaunch(Uri uri) async {
    try {
      return (_openUrlOverride ?? _openUrl)(
        uri,
        _isWeb ? LaunchMode.platformDefault : LaunchMode.externalApplication,
      );
    } catch (_) {
      return false;
    }
  }

  String _successMessage(InternalBuild build, Uri launchedUri) {
    final installInfo = build.installInfo;
    switch (installInfo.platform) {
      case InstallPlatform.ios:
        if (_isWeb && launchedUri.toString() == installInfo.manifestUrl) {
          return '已打开 iOS manifest 地址';
        }
        if (_isWeb && launchedUri.toString() == installInfo.downloadUrl) {
          return '已打开 iOS 安装包下载地址';
        }
        return installInfo.usesItmsServices
            ? '已打开 iOS manifest 安装'
            : '已打开 iOS 安装入口';
      case InstallPlatform.android:
        return '已打开 Android 安装包';
    }
  }
}

Future<bool> _openUrl(Uri uri, LaunchMode mode) {
  return launchUrl(uri, mode: mode);
}

Uri? _parseUri(String? value) {
  if (value == null || value.isEmpty) {
    return null;
  }
  final uri = Uri.tryParse(value);
  if (uri == null || !uri.hasScheme) {
    return null;
  }
  return uri;
}

List<Uri> _dedupe(List<Uri?> values) {
  final seen = <String>{};
  return [
    for (final value in values)
      if (value != null && seen.add(value.toString())) value,
  ];
}
