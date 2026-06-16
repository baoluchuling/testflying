import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:testflying/models/internal_build.dart';
import 'package:testflying/services/api_client.dart';
import 'package:testflying/services/install_launcher.dart';
import 'package:testflying/services/remote_testflight_service.dart';

class _FakeTransport implements ApiTransport {
  _FakeTransport(this.responses);

  final List<ApiResponse> responses;
  final requests = <ApiRequest>[];

  @override
  Future<ApiResponse> send(ApiRequest request) async {
    requests.add(request);
    return responses.removeAt(0);
  }
}

class _RecordingInstallLauncher implements InstallLauncher {
  final launchedBuildIds = <String>[];
  final launchedUrls = <String>[];

  @override
  Future<InstallLaunchResult> launch(InternalBuild build) async {
    launchedBuildIds.add(build.id);
    launchedUrls.add(build.installInfo.installUrl);
    return InstallLaunchResult.success(
        '已打开 ${build.installInfo.platform.label}');
  }
}

void main() {
  setUp(() {
    SharedPreferences.setMockInitialValues({});
  });

  test('remote service loads workspace through dto mapper', () async {
    final transport = _FakeTransport([
      ApiResponse(statusCode: 200, body: _workspaceJson()),
    ]);
    final service = RemoteTestFlightService(
      apiClient: _apiClient(transport),
      installLauncher: _RecordingInstallLauncher(),
    );

    final workspace = await service.loadWorkspace();

    expect(workspace.apps, hasLength(2));
    expect(workspace.builds.first.name, 'Aurora Mobile');
    expect(workspace.builds.first.installInfo.platform, InstallPlatform.ios);
    expect(workspace.builds.first.installInfo.usesItmsServices, isTrue);
    expect(workspace.builds.last.installInfo.platform, InstallPlatform.android);
    expect(workspace.currentDevice.name, 'iPhone 15 Pro');
    expect(
        workspace.renewalAccount.renewalTitle, 'Aurora Mobile 所属开发者账号 5 天后到期');
    expect(transport.requests.single.method, 'GET');
    expect(
      transport.requests.single.path,
      '/v1/test-distribution/workspace',
    );
  });

  test('remote service launches install entry then stores local install state',
      () async {
    final launcher = _RecordingInstallLauncher();
    final transport = _FakeTransport([
      ApiResponse(statusCode: 200, body: _workspaceJson()),
    ]);
    final service = RemoteTestFlightService(
      apiClient: _apiClient(transport),
      installLauncher: launcher,
    );
    final workspace = await service.loadWorkspace();
    final aurora = workspace.builds.first;

    final updated = await service.toggleInstallState(workspace, aurora);

    expect(launcher.launchedBuildIds, ['aurora']);
    expect(launcher.launchedUrls.single, startsWith('itms-services://'));
    expect(transport.requests, hasLength(1));
    expect(transport.requests.single.path, '/v1/test-distribution/workspace');
    expect(updated.builds.first.status, BuildStatus.installing);
    expect(
        updated.installTasks.any((task) => task.buildId == 'aurora'), isTrue);
  });

  test('remote service pauses installing build locally without server write',
      () async {
    final launcher = _RecordingInstallLauncher();
    final transport = _FakeTransport([
      ApiResponse(
        statusCode: 200,
        body: _workspaceJson(
          auroraStatus: 'installing',
          auroraProgress: .35,
          installTasks: [
            {
              'buildId': 'aurora',
              'deviceId': 'current-iphone',
              'progress': .35,
              'isPaused': false,
            },
          ],
        ),
      ),
    ]);
    final service = RemoteTestFlightService(
      apiClient: _apiClient(transport),
      installLauncher: launcher,
    );
    final workspace = await service.loadWorkspace();
    final aurora = workspace.builds.first;

    final updated = await service.toggleInstallState(workspace, aurora);

    expect(launcher.launchedBuildIds, isEmpty);
    expect(transport.requests, hasLength(1));
    expect(transport.requests.single.path, '/v1/test-distribution/workspace');
    expect(updated.builds.first.isPaused, isTrue);
    expect(updated.installTasks.single.isPaused, isTrue);
  });

  test('remote service reorders builds locally without server write', () async {
    final transport = _FakeTransport([
      ApiResponse(statusCode: 200, body: _workspaceJson()),
    ]);
    final service = RemoteTestFlightService(
      apiClient: _apiClient(transport),
      installLauncher: _RecordingInstallLauncher(),
    );
    final workspace = await service.loadWorkspace();

    final updated = await service.reorderVisibleBuilds(
      workspace,
      workspace.builds,
      0,
      2,
    );

    expect(transport.requests, hasLength(1));
    expect(transport.requests.single.path, '/v1/test-distribution/workspace');
    expect(updated.builds.map((build) => build.id), ['ops', 'aurora']);
  });

  test('remote service reapplies local state after workspace refresh',
      () async {
    final transport = _FakeTransport([
      ApiResponse(statusCode: 200, body: _workspaceJson()),
      ApiResponse(statusCode: 200, body: _workspaceJson()),
    ]);
    final service = RemoteTestFlightService(
      apiClient: _apiClient(transport),
      installLauncher: _RecordingInstallLauncher(),
    );
    final workspace = await service.loadWorkspace();
    final installed = await service.toggleInstallState(
      workspace,
      workspace.builds.first,
    );
    await service.reorderVisibleBuilds(installed, installed.builds, 0, 2);

    final refreshed = await service.loadWorkspace();

    expect(refreshed.builds.map((build) => build.id), ['ops', 'aurora']);
    expect(refreshed.builds.last.status, BuildStatus.installing);
    expect(refreshed.installTasks.single.buildId, 'aurora');
    expect(transport.requests, hasLength(2));
    expect(
      transport.requests.map((request) => request.path).toSet(),
      {'/v1/test-distribution/workspace'},
    );
  });
}

ApiClient _apiClient(_FakeTransport transport) {
  return ApiClient(
    environment: ApiEnvironment.development,
    context: const ApiClientContext(
      accessToken: 'token-123',
      deviceId: 'current-iphone',
      platform: 'ios',
    ),
    transport: transport,
  );
}

Map<String, Object?> _workspaceJson({
  String auroraStatus = 'available',
  double? auroraProgress,
  List<Map<String, Object?>> installTasks = const [],
}) {
  return {
    'apps': [
      {
        'id': 'aurora',
        'name': 'Aurora Mobile',
        'addedAt': '今天 11:08',
        'defaultChannel': 'dev',
        'iconKey': 'rocket',
        'iconColor': '#243D78',
      },
      {
        'id': 'ops',
        'name': 'Ops Console',
        'addedAt': '昨天 18:40',
        'defaultChannel': 'dev',
        'iconKey': 'terminal',
        'iconColor': '#17191F',
      },
    ],
    'builds': [
      {
        'id': 'aurora',
        'name': 'Aurora Mobile',
        'version': '2.3.0',
        'buildNumber': '23045',
        'channel': 'dev',
        'environment': '开发环境',
        'owner': '张三',
        'uploadedAt': '今天 11:08',
        'note': '登录链路开发环境',
        'status': auroraStatus,
        'iconKey': 'rocket',
        'iconColor': '#243D78',
        if (auroraProgress != null) 'progress': auroraProgress,
        'installInfo': {
          'platform': 'ios',
          'installUrl':
              'itms-services://?action=download-manifest&url=https%3A%2F%2Fdist.example.com%2Faurora.plist',
          'manifestUrl': 'https://dist.example.com/aurora.plist',
          'downloadUrl': 'https://dist.example.com/aurora.ipa',
          'minOsVersion': 'iOS 16.0',
          'expiresAt': '2026-06-20T12:00:00Z',
          'isInstallable': true,
        },
      },
      {
        'id': 'ops',
        'name': 'Ops Console',
        'version': '1.2.0',
        'buildNumber': '12001',
        'channel': 'dev',
        'environment': '开发环境',
        'owner': '王琳',
        'uploadedAt': '昨天 18:40',
        'note': '初始化版本发布',
        'status': 'available',
        'iconKey': 'terminal',
        'iconColor': '#17191F',
        'installInfo': {
          'platform': 'android',
          'installUrl': 'https://dist.example.com/ops.apk',
          'downloadUrl': 'https://dist.example.com/ops.apk',
          'minOsVersion': 'Android 10',
          'isInstallable': true,
        },
      },
    ],
    'devices': [
      {
        'id': 'current-iphone',
        'name': 'iPhone 15 Pro',
        'owner': '当前设备',
        'status': '已登记',
        'statusColor': '#22C55E',
        'detail': 'iOS 17.5.1 · 开发环境',
        'udid': '00008130-001E5D223A11801E',
        'osVersion': 'iOS 17.5.1',
        'certificateStatus': '企业签名有效',
        'lastInstalledAt': '今天 11:08',
        'isCurrent': true,
      },
    ],
    'developerAccounts': [
      {
        'id': 'apple-team-a',
        'appName': 'Aurora Mobile',
        'teamName': 'Apple Developer Team A',
        'remainingDays': 5,
        'renewalActionLabel': '去续费',
      },
    ],
    'notifications': [
      {
        'type': 'build',
        'section': '今天',
        'iconKey': 'upload',
        'title': 'Aurora Mobile 新构建已上传',
        'subtitle': '2.3.0 (23045) · 张三 · 今天 11:08',
        'tag': '待更新',
        'tagColor': '#2478FF',
      },
    ],
    'installTasks': installTasks,
    'sortOrder': {
      'buildIds': ['aurora', 'ops'],
    },
    'profile': {
      'name': '张三',
      'initial': '张',
      'subtitle': '测试管理员 · 移动端测试组',
      'metrics': [],
      'actions': [],
      'preferences': [],
    },
  };
}
