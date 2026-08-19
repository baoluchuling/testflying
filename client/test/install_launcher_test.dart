import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:testflying/models/internal_build.dart';
import 'package:testflying/services/install_launcher.dart';
import 'package:url_launcher/url_launcher.dart';

class _RecordingUrlOpener {
  _RecordingUrlOpener(this.results);

  final List<bool> results;
  final requests = <_OpenRequest>[];

  Future<bool> open(Uri uri, LaunchMode mode) async {
    requests.add(_OpenRequest(uri, mode));
    return results.removeAt(0);
  }
}

class _OpenRequest {
  const _OpenRequest(this.uri, this.mode);

  final Uri uri;
  final LaunchMode mode;
}

void main() {
  test('native launcher opens install url with external application mode',
      () async {
    final opener = _RecordingUrlOpener([true]);
    final launcher = UrlInstallLauncher.forTesting(
      openUrl: opener.open,
      isWeb: false,
    );

    final result = await launcher.launch(_build());

    expect(result.success, isTrue);
    expect(
        opener.requests.single.uri.toString(), startsWith('itms-services://'));
    expect(opener.requests.single.mode, LaunchMode.externalApplication);
  });

  test('web launcher falls back from itms services to manifest url', () async {
    final opener = _RecordingUrlOpener([false, true]);
    final launcher = UrlInstallLauncher.forTesting(
      openUrl: opener.open,
      isWeb: true,
    );

    final result = await launcher.launch(_build());

    expect(result.success, isTrue);
    expect(result.message, '已打开 iOS manifest 地址');
    expect(opener.requests.map((request) => request.uri.toString()), [
      _installUrl,
      _manifestUrl,
    ]);
    expect(
      opener.requests.map((request) => request.mode).toSet(),
      {LaunchMode.platformDefault},
    );
  });

  test('web launcher opens android download url when install url fails',
      () async {
    final opener = _RecordingUrlOpener([false, true]);
    final launcher = UrlInstallLauncher.forTesting(
      openUrl: opener.open,
      isWeb: true,
    );

    final result = await launcher.launch(
      _build(
        installInfo: const BuildInstallInfo(
          platform: InstallPlatform.android,
          installUrl: 'internal-dist://android/build-1',
          downloadUrl: 'https://dist.example.test/app.apk',
        ),
      ),
    );

    expect(result.success, isTrue);
    expect(opener.requests.map((request) => request.uri.toString()), [
      'internal-dist://android/build-1',
      'https://dist.example.test/app.apk',
    ]);
  });

  test('launcher rejects unavailable builds without opening url', () async {
    final opener = _RecordingUrlOpener([true]);
    final launcher = UrlInstallLauncher.forTesting(
      openUrl: opener.open,
      isWeb: false,
    );

    final result = await launcher.launch(
      _build(
        installInfo: const BuildInstallInfo(
          platform: InstallPlatform.ios,
          installUrl: _installUrl,
          isInstallable: false,
          unavailableReason: '签名已过期',
        ),
      ),
    );

    expect(result.success, isFalse);
    expect(result.message, '签名已过期');
    expect(opener.requests, isEmpty);
  });
}

InternalBuild _build({BuildInstallInfo installInfo = _iosInstallInfo}) {
  return InternalBuild(
    id: 'aurora',
    name: 'Aurora Mobile',
    version: '1.0.0',
    buildNumber: '100',
    channel: BuildChannel.dev,
    environment: 'development',
    owner: '',
    uploadedAt: '今天 10:00',
    note: '',
    status: BuildStatus.available,
    icon: Icons.rocket_launch_rounded,
    iconColor: const Color(0xFF2478FF),
    installInfo: installInfo,
  );
}

const _installUrl =
    'itms-services://?action=download-manifest&url=https%3A%2F%2Fdist.example.test%2Fmanifest.plist';
const _manifestUrl = 'https://dist.example.test/manifest.plist';

const _iosInstallInfo = BuildInstallInfo(
  platform: InstallPlatform.ios,
  installUrl: _installUrl,
  manifestUrl: _manifestUrl,
  downloadUrl: 'https://dist.example.test/app.ipa',
);
