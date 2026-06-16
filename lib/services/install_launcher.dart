import 'package:testflying/models/internal_build.dart';
import 'package:url_launcher/url_launcher.dart';

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
  const UrlInstallLauncher();

  @override
  Future<InstallLaunchResult> launch(InternalBuild build) async {
    final installInfo = build.installInfo;
    if (!installInfo.isInstallable) {
      return InstallLaunchResult.failure(
        installInfo.unavailableReason ?? '${build.name} 当前不可安装',
      );
    }

    final uri = Uri.tryParse(installInfo.installUrl);
    if (uri == null || !uri.hasScheme) {
      return InstallLaunchResult.failure('${build.name} 安装链接无效');
    }

    try {
      final launched = await launchUrl(
        uri,
        mode: LaunchMode.externalApplication,
      );
      if (!launched) {
        return InstallLaunchResult.failure(
          '${installInfo.platform.label} 安装入口打开失败',
        );
      }
    } catch (_) {
      return InstallLaunchResult.failure(
        '${installInfo.platform.label} 安装入口打开失败',
      );
    }

    return InstallLaunchResult.success(_successMessage(build));
  }

  String _successMessage(InternalBuild build) {
    final installInfo = build.installInfo;
    switch (installInfo.platform) {
      case InstallPlatform.ios:
        return installInfo.usesItmsServices
            ? '已打开 iOS manifest 安装'
            : '已打开 iOS 安装入口';
      case InstallPlatform.android:
        return '已打开 Android 安装包';
    }
  }
}
