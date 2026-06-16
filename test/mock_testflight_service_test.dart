import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:testflying/models/internal_build.dart';
import 'package:testflying/services/install_launcher.dart';
import 'package:testflying/services/mock_testflight_service.dart';
import 'package:testflying/services/testflight_service.dart';

class _RecordingInstallLauncher implements InstallLauncher {
  _RecordingInstallLauncher({this.succeeds = true});

  final bool succeeds;
  final launchedBuildIds = <String>[];
  final launchedUrls = <String>[];

  @override
  Future<InstallLaunchResult> launch(InternalBuild build) async {
    launchedBuildIds.add(build.id);
    launchedUrls.add(build.installInfo.installUrl);
    if (!succeeds) {
      return const InstallLaunchResult.failure('安装入口打开失败');
    }
    return InstallLaunchResult.success(
        '已打开 ${build.installInfo.platform.label}');
  }
}

void main() {
  setUp(() {
    SharedPreferences.setMockInitialValues({});
  });

  test('mock service provides workspace snapshot', () async {
    final TestFlightService service = MockTestFlightService();
    final workspace = await service.loadWorkspace();

    expect(workspace.apps, hasLength(7));
    expect(workspace.builds.first.name, 'Aurora Mobile');
    expect(workspace.currentDevice.name, 'iPhone 15 Pro');
    expect(
        workspace.renewalAccount.renewalTitle, 'Aurora Mobile 所属开发者账号 5 天后到期');
    expect(workspace.notifications, hasLength(5));
    expect(workspace.profile.name, '张三');
  });

  test('mock service toggles install task state', () async {
    final service = MockTestFlightService();
    final workspace = await service.loadWorkspace();
    final aurora = workspace.builds.first;

    final paused = await service.toggleInstallState(workspace, aurora);

    expect(paused.builds.first.status, BuildStatus.installing);
    expect(paused.builds.first.isPaused, isTrue);
    expect(paused.installTasks.single.isPaused, isTrue);
  });

  test('mock service launches iOS manifest install before updating state',
      () async {
    final launcher = _RecordingInstallLauncher();
    final service = MockTestFlightService(installLauncher: launcher);
    final workspace = await service.loadWorkspace();
    final dataflow =
        workspace.builds.firstWhere((build) => build.id == 'dataflow');

    final updated = await service.toggleInstallState(workspace, dataflow);
    final updatedDataflow =
        updated.builds.firstWhere((build) => build.id == 'dataflow');

    expect(launcher.launchedBuildIds, ['dataflow']);
    expect(launcher.launchedUrls.single, startsWith('itms-services://'));
    expect(launcher.launchedUrls.single, contains('manifest'));
    expect(dataflow.installInfo.platform, InstallPlatform.ios);
    expect(dataflow.installInfo.usesItmsServices, isTrue);
    expect(updatedDataflow.status, BuildStatus.installing);
    expect(
      updated.installTasks.any((task) => task.buildId == 'dataflow'),
      isTrue,
    );
  });

  test('mock service launches Android apk install before updating state',
      () async {
    final launcher = _RecordingInstallLauncher();
    final service = MockTestFlightService(installLauncher: launcher);
    final workspace = await service.loadWorkspace();
    final ops = workspace.builds.firstWhere((build) => build.id == 'ops');

    final updated = await service.toggleInstallState(workspace, ops);
    final updatedOps = updated.builds.firstWhere((build) => build.id == 'ops');

    expect(launcher.launchedBuildIds, ['ops']);
    expect(launcher.launchedUrls.single, endsWith('.apk'));
    expect(ops.installInfo.platform, InstallPlatform.android);
    expect(updatedOps.status, BuildStatus.installing);
    expect(
      updated.installTasks.any((task) => task.buildId == 'ops'),
      isTrue,
    );
  });

  test('mock service keeps state when install launcher fails', () async {
    final service = MockTestFlightService(
      installLauncher: _RecordingInstallLauncher(succeeds: false),
    );
    final workspace = await service.loadWorkspace();
    final dataflow =
        workspace.builds.firstWhere((build) => build.id == 'dataflow');

    await expectLater(
      service.toggleInstallState(workspace, dataflow),
      throwsA(isA<InstallLaunchException>()),
    );
    final reloaded = await service.loadWorkspace();

    expect(reloaded.builds.firstWhere((build) => build.id == 'dataflow').status,
        BuildStatus.updateAvailable);
  });

  test('mock service reorders visible builds', () async {
    final service = MockTestFlightService();
    final workspace = await service.loadWorkspace();

    final reordered = await service.reorderVisibleBuilds(
      workspace,
      workspace.builds,
      0,
      2,
    );

    expect(reordered.builds.map((build) => build.id).take(3),
        ['dataflow', 'aurora', 'insight']);
    expect(reordered.sortOrder.buildIds.take(3),
        ['dataflow', 'aurora', 'insight']);
  });

  test('mock service persists mutable workspace state', () async {
    final service = MockTestFlightService();
    final workspace = await service.loadWorkspace();
    final reordered = await service.reorderVisibleBuilds(
      workspace,
      workspace.builds,
      0,
      2,
    );
    final aurora = reordered.builds.firstWhere((build) => build.id == 'aurora');

    await service.toggleInstallState(reordered, aurora);
    final reloaded = await MockTestFlightService().loadWorkspace();
    final reloadedAurora =
        reloaded.builds.firstWhere((build) => build.id == 'aurora');

    expect(reloaded.builds.map((build) => build.id).take(3),
        ['dataflow', 'aurora', 'insight']);
    expect(
        reloaded.sortOrder.buildIds.take(3), ['dataflow', 'aurora', 'insight']);
    expect(reloadedAurora.status, BuildStatus.installing);
    expect(reloadedAurora.isPaused, isTrue);
    expect(reloaded.installTasks.single.isPaused, isTrue);
  });
}
