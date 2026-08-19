import 'package:testflying/models/internal_build.dart';
import 'package:testflying/models/workspace_data.dart';
import 'package:testflying/services/api_client.dart';
import 'package:testflying/services/install_launcher.dart';
import 'package:testflying/services/remote_workspace_dto.dart';
import 'package:testflying/services/testflight_service.dart';
import 'package:testflying/services/workspace_preferences_store.dart';

class RemoteTestFlightService implements TestFlightService {
  RemoteTestFlightService({
    required ApiClient apiClient,
    WorkspacePreferencesStore preferencesStore =
        const WorkspacePreferencesStore(),
    InstallLauncher installLauncher = const UrlInstallLauncher(),
  })  : _apiClient = apiClient,
        _preferencesStore = preferencesStore,
        _installLauncher = installLauncher;

  final ApiClient _apiClient;
  final WorkspacePreferencesStore _preferencesStore;
  final InstallLauncher _installLauncher;

  @override
  Future<TestFlightWorkspace> loadWorkspace() async {
    final response = await _apiClient.get('/v1/test-distribution/workspace');
    final workspace = RemoteWorkspaceDto.fromJson(response).toDomain();
    final preferences = await _preferencesStore.load();
    return preferences.applyTo(workspace);
  }

  @override
  Future<TestFlightWorkspace> toggleInstallState(
    TestFlightWorkspace workspace,
    InternalBuild selectedBuild,
  ) async {
    if (selectedBuild.status != BuildStatus.installing) {
      final launchResult = await _installLauncher.launch(selectedBuild);
      if (!launchResult.success) {
        throw InstallLaunchException(launchResult.message);
      }
    }

    final updatedWorkspace = _applyInstallState(workspace, selectedBuild);
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
    final updatedWorkspace = _applyVisibleBuildReorder(
      workspace,
      visibleBuilds,
      oldIndex,
      newIndex,
    );
    await _preferencesStore.save(updatedWorkspace);
    return updatedWorkspace;
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
