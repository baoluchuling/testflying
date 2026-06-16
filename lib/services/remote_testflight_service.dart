import 'package:testflying/models/internal_build.dart';
import 'package:testflying/models/workspace_data.dart';
import 'package:testflying/services/api_client.dart';
import 'package:testflying/services/install_launcher.dart';
import 'package:testflying/services/remote_workspace_dto.dart';
import 'package:testflying/services/testflight_service.dart';

class RemoteTestFlightService implements TestFlightService {
  const RemoteTestFlightService({
    required ApiClient apiClient,
    InstallLauncher installLauncher = const UrlInstallLauncher(),
  })  : _apiClient = apiClient,
        _installLauncher = installLauncher;

  final ApiClient _apiClient;
  final InstallLauncher _installLauncher;

  @override
  Future<TestFlightWorkspace> loadWorkspace() async {
    final response = await _apiClient.get('/v1/test-distribution/workspace');
    return RemoteWorkspaceDto.fromJson(response).toDomain();
  }

  @override
  Future<TestFlightWorkspace> toggleInstallState(
    TestFlightWorkspace workspace,
    InternalBuild selectedBuild,
  ) async {
    if (selectedBuild.status == BuildStatus.installing) {
      final action = selectedBuild.isPaused ? 'resume' : 'pause';
      final fallback = _applyInstallState(workspace, selectedBuild);
      final response = await _apiClient.patch(
        '/v1/test-distribution/install-tasks/${selectedBuild.id}',
        body: {
          'action': action,
          'buildId': selectedBuild.id,
          'deviceId': workspace.currentDevice.id,
        },
      );
      return _workspaceOrFallback(response, fallback);
    }

    final launchResult = await _installLauncher.launch(selectedBuild);
    if (!launchResult.success) {
      throw InstallLaunchException(launchResult.message);
    }

    final fallback = _applyInstallState(workspace, selectedBuild);
    final response = await _apiClient.post(
      '/v1/test-distribution/builds/${selectedBuild.id}/install-tasks',
      body: {
        'action': 'start',
        'buildId': selectedBuild.id,
        'deviceId': workspace.currentDevice.id,
        'platform': selectedBuild.installInfo.platform.name,
      },
    );
    return _workspaceOrFallback(response, fallback);
  }

  @override
  Future<TestFlightWorkspace> reorderVisibleBuilds(
    TestFlightWorkspace workspace,
    List<InternalBuild> visibleBuilds,
    int oldIndex,
    int newIndex,
  ) async {
    final fallback = _applyVisibleBuildReorder(
      workspace,
      visibleBuilds,
      oldIndex,
      newIndex,
    );
    final response = await _apiClient.put(
      '/v1/test-distribution/users/me/build-sort-order',
      body: {
        'buildIds': fallback.sortOrder.buildIds,
      },
    );
    return _workspaceOrFallback(response, fallback);
  }

  TestFlightWorkspace _workspaceOrFallback(
    Map<String, Object?> response,
    TestFlightWorkspace fallback,
  ) {
    if (response.containsKey('workspace') || response.containsKey('builds')) {
      return RemoteWorkspaceDto.fromJson(response).toDomain();
    }
    return fallback;
  }

  TestFlightWorkspace _applyInstallState(
    TestFlightWorkspace workspace,
    InternalBuild selectedBuild,
  ) {
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

    return workspace.copyWith(
      builds: builds,
      installTasks: installTasks,
    );
  }

  TestFlightWorkspace _applyVisibleBuildReorder(
    TestFlightWorkspace workspace,
    List<InternalBuild> visibleBuilds,
    int oldIndex,
    int newIndex,
  ) {
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

    return workspace.copyWith(
      builds: builds,
      sortOrder:
          AppSortOrder(buildIds: builds.map((build) => build.id).toList()),
    );
  }
}
