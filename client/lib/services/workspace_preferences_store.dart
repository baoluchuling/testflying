import 'dart:convert';

import 'package:shared_preferences/shared_preferences.dart';
import 'package:testflying/models/internal_build.dart';
import 'package:testflying/models/workspace_data.dart';

class WorkspacePreferencesStore {
  const WorkspacePreferencesStore();

  static const _storageKey = 'testflying.workspace.preferences.v1';

  Future<WorkspacePreferencesSnapshot> load() async {
    final preferences = await SharedPreferences.getInstance();
    final rawValue = preferences.getString(_storageKey);
    if (rawValue == null || rawValue.isEmpty) {
      return const WorkspacePreferencesSnapshot.empty();
    }

    try {
      final decoded = jsonDecode(rawValue);
      if (decoded is! Map<String, Object?>) {
        return const WorkspacePreferencesSnapshot.empty();
      }
      return WorkspacePreferencesSnapshot.fromJson(decoded);
    } on FormatException {
      return const WorkspacePreferencesSnapshot.empty();
    }
  }

  Future<void> save(TestFlightWorkspace workspace) async {
    final preferences = await SharedPreferences.getInstance();
    final snapshot = WorkspacePreferencesSnapshot.fromWorkspace(workspace);
    await preferences.setString(_storageKey, jsonEncode(snapshot.toJson()));
  }

  Future<void> clear() async {
    final preferences = await SharedPreferences.getInstance();
    await preferences.remove(_storageKey);
  }
}

class WorkspacePreferencesSnapshot {
  const WorkspacePreferencesSnapshot({
    required this.sortOrder,
    required this.buildStates,
    required this.installTasks,
  });

  const WorkspacePreferencesSnapshot.empty()
      : sortOrder = const [],
        buildStates = const {},
        installTasks = const [];

  factory WorkspacePreferencesSnapshot.fromWorkspace(
    TestFlightWorkspace workspace,
  ) {
    return WorkspacePreferencesSnapshot(
      sortOrder: workspace.sortOrder.buildIds,
      buildStates: {
        for (final build in workspace.builds)
          build.id: PersistedBuildState.fromBuild(build),
      },
      installTasks: [
        for (final task in workspace.installTasks)
          PersistedInstallTask.fromTask(task),
      ],
    );
  }

  factory WorkspacePreferencesSnapshot.fromJson(Map<String, Object?> json) {
    final rawSortOrder = json['sortOrder'];
    final rawBuildStates = json['buildStates'];
    final rawInstallTasks = json['installTasks'];
    final buildStates = <String, PersistedBuildState>{};
    if (rawBuildStates is Map) {
      for (final entry in rawBuildStates.entries) {
        final key = entry.key;
        final value = entry.value;
        if (key is String && value is Map<String, Object?>) {
          buildStates[key] = PersistedBuildState.fromJson(value);
        }
      }
    }

    return WorkspacePreferencesSnapshot(
      sortOrder: rawSortOrder is List
          ? rawSortOrder.whereType<String>().toList(growable: false)
          : const [],
      buildStates: buildStates,
      installTasks: rawInstallTasks is List
          ? rawInstallTasks
              .whereType<Map<String, Object?>>()
              .map(PersistedInstallTask.fromJson)
              .toList(growable: false)
          : const [],
    );
  }

  final List<String> sortOrder;
  final Map<String, PersistedBuildState> buildStates;
  final List<PersistedInstallTask> installTasks;

  TestFlightWorkspace applyTo(TestFlightWorkspace workspace) {
    final buildsById = {
      for (final build in workspace.builds)
        build.id: _applyBuildState(build, buildStates[build.id]),
    };
    final seenBuildIds = <String>{};
    final orderedIds = [
      for (final id in sortOrder)
        if (buildsById.containsKey(id) && seenBuildIds.add(id)) id,
      for (final build in workspace.builds)
        if (seenBuildIds.add(build.id)) build.id,
    ];
    final builds = [
      for (final id in orderedIds) buildsById[id]!,
    ];
    final validBuildIds = builds.map((build) => build.id).toSet();
    final validDeviceIds = workspace.devices.map((device) => device.id).toSet();
    final persistedInstallTasks = installTasks.isEmpty
        ? workspace.installTasks
        : [
            for (final task in installTasks)
              if (validBuildIds.contains(task.buildId) &&
                  validDeviceIds.contains(task.deviceId))
                InstallTask(
                  buildId: task.buildId,
                  deviceId: task.deviceId,
                  progress: task.progress,
                  isPaused: task.isPaused,
                ),
          ];

    if (sortOrder.isEmpty && buildStates.isEmpty && installTasks.isEmpty) {
      return workspace;
    }

    return workspace.copyWith(
      builds: builds,
      installTasks: persistedInstallTasks,
      sortOrder:
          AppSortOrder(buildIds: builds.map((build) => build.id).toList()),
    );
  }

  Map<String, Object?> toJson() {
    return {
      'sortOrder': sortOrder,
      'buildStates': {
        for (final entry in buildStates.entries)
          entry.key: entry.value.toJson(),
      },
      'installTasks': [
        for (final task in installTasks) task.toJson(),
      ],
    };
  }

  InternalBuild _applyBuildState(
    InternalBuild build,
    PersistedBuildState? state,
  ) {
    if (state == null) {
      return build;
    }
    return build.copyWith(
      status: state.status,
      progress: state.progress,
      isPaused: state.isPaused,
    );
  }
}

class PersistedBuildState {
  const PersistedBuildState({
    required this.status,
    required this.progress,
    required this.isPaused,
  });

  factory PersistedBuildState.fromBuild(InternalBuild build) {
    return PersistedBuildState(
      status: build.status,
      progress: build.progress,
      isPaused: build.isPaused,
    );
  }

  factory PersistedBuildState.fromJson(Map<String, Object?> json) {
    final progress = json['progress'];
    return PersistedBuildState(
      status: BuildStatus.values.firstWhere(
        (status) => status.name == json['status'],
        orElse: () => BuildStatus.available,
      ),
      progress: progress is num ? progress.clamp(0.0, 1.0).toDouble() : null,
      isPaused: json['isPaused'] == true,
    );
  }

  final BuildStatus status;
  final double? progress;
  final bool isPaused;

  Map<String, Object?> toJson() {
    return {
      'status': status.name,
      'progress': progress,
      'isPaused': isPaused,
    };
  }
}

class PersistedInstallTask {
  const PersistedInstallTask({
    required this.buildId,
    required this.deviceId,
    required this.progress,
    required this.isPaused,
  });

  factory PersistedInstallTask.fromTask(InstallTask task) {
    return PersistedInstallTask(
      buildId: task.buildId,
      deviceId: task.deviceId,
      progress: task.progress,
      isPaused: task.isPaused,
    );
  }

  factory PersistedInstallTask.fromJson(Map<String, Object?> json) {
    final progress = json['progress'];
    return PersistedInstallTask(
      buildId: json['buildId'] is String ? json['buildId'] as String : '',
      deviceId: json['deviceId'] is String ? json['deviceId'] as String : '',
      progress: progress is num ? progress.clamp(0.0, 1.0).toDouble() : 0,
      isPaused: json['isPaused'] == true,
    );
  }

  final String buildId;
  final String deviceId;
  final double progress;
  final bool isPaused;

  Map<String, Object?> toJson() {
    return {
      'buildId': buildId,
      'deviceId': deviceId,
      'progress': progress,
      'isPaused': isPaused,
    };
  }
}
