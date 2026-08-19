import 'package:testflying/models/internal_build.dart';
import 'package:testflying/models/workspace_data.dart';

abstract class TestFlightService {
  Future<TestFlightWorkspace> loadWorkspace();

  Future<TestFlightWorkspace> toggleInstallState(
    TestFlightWorkspace workspace,
    InternalBuild selectedBuild,
  );

  Future<TestFlightWorkspace> reorderVisibleBuilds(
    TestFlightWorkspace workspace,
    List<InternalBuild> visibleBuilds,
    int oldIndex,
    int newIndex,
  );
}
