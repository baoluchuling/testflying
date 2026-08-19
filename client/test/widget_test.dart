import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:testflying/main.dart';
import 'package:testflying/models/internal_build.dart';
import 'package:testflying/models/workspace_data.dart';
import 'package:testflying/pages/home.dart';
import 'package:testflying/services/mock_testflight_service.dart';
import 'package:testflying/services/testflight_service.dart';

Future<void> pumpApp(WidgetTester tester) async {
  SharedPreferences.setMockInitialValues({});
  await tester.pumpWidget(const MyApp());
  await tester.pumpAndSettle();
}

Future<void> pumpHomeWithService(
  WidgetTester tester,
  TestFlightService service,
) async {
  await tester.pumpWidget(
    MaterialApp(home: HomePage(service: service)),
  );
}

class _PendingWorkspaceService implements TestFlightService {
  _PendingWorkspaceService(this.completer);

  final Completer<TestFlightWorkspace> completer;

  @override
  Future<TestFlightWorkspace> loadWorkspace() => completer.future;

  @override
  Future<TestFlightWorkspace> reorderVisibleBuilds(
    TestFlightWorkspace workspace,
    List<InternalBuild> visibleBuilds,
    int oldIndex,
    int newIndex,
  ) async {
    return workspace;
  }

  @override
  Future<TestFlightWorkspace> toggleInstallState(
    TestFlightWorkspace workspace,
    InternalBuild selectedBuild,
  ) async {
    return workspace;
  }
}

class _RetryWorkspaceService extends _PendingWorkspaceService {
  _RetryWorkspaceService()
      : workspace = MockTestFlightService.seedWorkspace(),
        super(Completer<TestFlightWorkspace>());

  final TestFlightWorkspace workspace;
  int calls = 0;

  @override
  Future<TestFlightWorkspace> loadWorkspace() async {
    calls += 1;
    if (calls == 1) {
      throw StateError('workspace unavailable');
    }
    return workspace;
  }
}

class _StaticWorkspaceService extends _PendingWorkspaceService {
  _StaticWorkspaceService(this.workspace)
      : super(Completer<TestFlightWorkspace>());

  final TestFlightWorkspace workspace;

  @override
  Future<TestFlightWorkspace> loadWorkspace() async => workspace;
}

Future<void> tapBottomTab(WidgetTester tester, int index) async {
  final nav = find.byType(BottomNavigationBar);
  final navigationBar = tester.widget<BottomNavigationBar>(nav);
  final topLeft = tester.getTopLeft(nav);
  final size = tester.getSize(nav);
  final itemCount = navigationBar.items.length;

  await tester.tapAt(
    Offset(
      topLeft.dx + size.width * (index + .5) / itemCount,
      topLeft.dy + size.height / 2,
    ),
  );
  await tester.pumpAndSettle();
}

Future<void> scrollUntilTextVisible(WidgetTester tester, String text) async {
  await tester.scrollUntilVisible(
    find.text(text),
    420,
    scrollable: find.byType(Scrollable).first,
  );
  await tester.pumpAndSettle();
}

Future<void> expectCollapsibleTabTitle(
  WidgetTester tester,
  String prefix,
) async {
  final titleBar = tester.widget<SliverAppBar>(
    find.byKey(ValueKey('$prefix-title-bar')),
  );
  expect(titleBar.pinned, isTrue);
  expect(titleBar.centerTitle, isTrue);
  expect(titleBar.expandedHeight, 92);
  expect(find.byKey(ValueKey('$prefix-large-title')), findsOneWidget);
  expect(find.byKey(ValueKey('$prefix-small-title')), findsNothing);
  expect(
    tester.getTopLeft(find.byKey(ValueKey('$prefix-large-title'))).dx,
    closeTo(16, 1),
  );

  await tester.drag(
    find.byKey(ValueKey('$prefix-scroll-view')),
    const Offset(0, -28),
  );
  await tester.pumpAndSettle();

  final midLargeTitle = tester.widget<Opacity>(
    find.byKey(ValueKey('$prefix-large-title-opacity')),
  );
  final midSmallTitle = tester.widget<Opacity>(
    find.byKey(ValueKey('$prefix-small-title-opacity')),
  );
  final midSmallTitleFlip = tester.widget<Transform>(
    find.byKey(ValueKey('$prefix-small-title-flip')),
  );
  expect(midLargeTitle.opacity, allOf(greaterThan(0), lessThanOrEqualTo(.5)));
  expect(midSmallTitle.opacity, allOf(greaterThan(0), lessThan(1)));
  expect(midSmallTitleFlip.alignment, Alignment.bottomCenter);
  expect(midSmallTitleFlip.transform.storage[5], lessThan(.98));

  await tester.drag(
    find.byKey(ValueKey('$prefix-scroll-view')),
    const Offset(0, -180),
  );
  await tester.pumpAndSettle();

  expect(find.byKey(ValueKey('$prefix-small-title')), findsOneWidget);
  expect(find.byKey(ValueKey('$prefix-large-title')), findsNothing);
}

void main() {
  testWidgets('home shows loading state while workspace is pending',
      (tester) async {
    final completer = Completer<TestFlightWorkspace>();

    await pumpHomeWithService(tester, _PendingWorkspaceService(completer));

    expect(find.byKey(const ValueKey('workspace-loading')), findsOneWidget);
    expect(find.text('正在加载测试工作台'), findsOneWidget);
    expect(
      find.byKey(const ValueKey('workspace-loading-indicator')),
      findsOneWidget,
    );

    completer.complete(MockTestFlightService.seedWorkspace());
    await tester.pumpAndSettle();

    expect(find.text('内测工具'), findsOneWidget);
    expect(find.byKey(const ValueKey('workspace-loading')), findsNothing);
  });

  testWidgets('home can retry after workspace load failure', (tester) async {
    final service = _RetryWorkspaceService();

    await pumpHomeWithService(tester, service);
    await tester.pumpAndSettle();

    expect(find.byKey(const ValueKey('workspace-error')), findsOneWidget);
    expect(find.text('工作台加载失败'), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('workspace-retry')));
    await tester.pumpAndSettle();

    expect(find.text('内测工具'), findsOneWidget);
    expect(find.byKey(const ValueKey('workspace-error')), findsNothing);
  });

  testWidgets('home shows empty state when there are no builds',
      (tester) async {
    final workspace = MockTestFlightService.seedWorkspace().copyWith(
      apps: const [],
      builds: const [],
      installTasks: const [],
      sortOrder: const AppSortOrder(buildIds: []),
    );

    await pumpHomeWithService(tester, _StaticWorkspaceService(workspace));
    await tester.pumpAndSettle();

    expect(find.byKey(const ValueKey('build-empty-state')), findsOneWidget);
    expect(find.text('暂无应用构建'), findsOneWidget);
    expect(find.text('上传新包后会自动出现在这里'), findsOneWidget);
    expect(find.byKey(const ValueKey('build-sort-entry')), findsNothing);
  });

  testWidgets('home shows internal test console sections', (tester) async {
    await pumpApp(tester);

    expect(find.text('内测工具'), findsOneWidget);
    expect(find.text('Aurora Mobile 所属开发者账号 5 天后到期'), findsOneWidget);
    expect(find.text('账号续费预警'), findsOneWidget);
    expect(find.byIcon(Icons.warning_amber_rounded), findsOneWidget);
    final renewalTitle = tester.widget<Text>(
      find.text('Aurora Mobile 所属开发者账号 5 天后到期'),
    );
    expect(renewalTitle.maxLines, 2);
    expect(renewalTitle.softWrap, isTrue);
    expect(renewalTitle.overflow, TextOverflow.visible);
    expect(find.text('去续费'), findsOneWidget);
    expect(find.text('设备与证书'), findsOneWidget);
    expect(find.text('iPhone 15 Pro 已登记 · 开发者账号续费待处理'), findsOneWidget);
    expect(find.text('查看设备'), findsOneWidget);
    expect(find.text('应用总览'), findsOneWidget);
    expect(find.text('全部应用'), findsOneWidget);
    expect(find.byTooltip('收起应用总览'), findsOneWidget);
    expect(find.text('收起为标签'), findsNothing);
    expect(find.text('当前设备'), findsNothing);
    expect(find.text('今日构建'), findsNothing);
    expect(find.byType(ChoiceChip), findsNothing);
    expect(find.text('应用列表'), findsOneWidget);
    expect(find.byKey(const ValueKey('build-sort-entry')), findsOneWidget);
    expect(find.text('开发环境'), findsWidgets);
    expect(find.text('线上环境'), findsWidgets);
    expect(find.text('已安装'), findsWidgets);
    expect(find.text('Aurora Mobile'), findsOneWidget);
    expect(find.text('安装中'), findsWidgets);
    expect(find.text('35%'), findsNothing);
    expect(find.byType(LinearProgressIndicator), findsNothing);
    final downloadProgress = tester.widget<CircularProgressIndicator>(
      find.byKey(const ValueKey('download-progress-aurora')),
    );
    expect(downloadProgress.value, closeTo(.35, .001));
    await scrollUntilTextVisible(tester, 'DataFlow');
    expect(find.text('DataFlow'), findsOneWidget);
    expect(find.text('待更新'), findsWidgets);
    await scrollUntilTextVisible(tester, 'Ops Console');
    expect(find.text('Ops Console'), findsOneWidget);
    expect(find.text('可安装'), findsWidgets);
  });

  testWidgets('overview filters list and toggles selected state',
      (tester) async {
    await pumpApp(tester);

    await tester.ensureVisible(find.byKey(const ValueKey('overview-updates')));
    await tester.pumpAndSettle();
    final scrollable = tester.widget<Scrollable>(find.byType(Scrollable).first);
    final position = scrollable.controller!.position;
    final beforeUpdatesTapOffset = position.pixels;
    await tester.tap(find.byKey(const ValueKey('overview-updates')));
    await tester.pumpAndSettle();

    expect(position.pixels, beforeUpdatesTapOffset);
    expect(find.text('DataFlow'), findsOneWidget);
    expect(find.text('Insight Pro'), findsOneWidget);
    expect(find.text('Aurora Mobile'), findsNothing);
    expect(find.text('Ops Console'), findsNothing);
    final selectedUpdatesMetric = tester.widget<Material>(
      find
          .descendant(
            of: find.byKey(const ValueKey('overview-updates')),
            matching: find.byType(Material),
          )
          .first,
    );
    expect(selectedUpdatesMetric.color, const Color(0xFF2478FF));

    await tester.ensureVisible(find.byKey(const ValueKey('overview-updates')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('overview-updates')));
    await tester.pumpAndSettle();
    expect(find.text('Aurora Mobile'), findsOneWidget);
    expect(find.text('Ops Console'), findsOneWidget);
    final unselectedUpdatesMetric = tester.widget<Material>(
      find
          .descendant(
            of: find.byKey(const ValueKey('overview-updates')),
            matching: find.byType(Material),
          )
          .first,
    );
    expect(unselectedUpdatesMetric.color, const Color(0xFFF2F3F6));

    await tester
        .ensureVisible(find.byKey(const ValueKey('overview-installed')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('overview-installed')));
    await tester.pumpAndSettle();
    expect(find.text('ReaderKit Demo'), findsOneWidget);
    expect(find.text('Insight Pro'), findsNothing);
    final selectedInstalledMetric = tester.widget<Material>(
      find
          .descendant(
            of: find.byKey(const ValueKey('overview-installed')),
            matching: find.byType(Material),
          )
          .first,
    );
    expect(selectedInstalledMetric.color, const Color(0xFF2478FF));

    await tester.ensureVisible(find.byKey(const ValueKey('overview-all')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('overview-all')));
    await tester.pumpAndSettle();
    expect(find.text('Aurora Mobile'), findsOneWidget);
    expect(find.text('ReaderKit Demo'), findsOneWidget);
    final allMetric = tester.widget<Material>(
      find
          .descendant(
            of: find.byKey(const ValueKey('overview-all')),
            matching: find.byType(Material),
          )
          .first,
    );
    expect(allMetric.color, const Color(0xFFF2F3F6));
  });

  testWidgets('overview all restores list without scrolling', (tester) async {
    await pumpApp(tester);

    await tester.ensureVisible(find.byKey(const ValueKey('overview-all')));
    await tester.pumpAndSettle();
    final scrollable = tester.widget<Scrollable>(find.byType(Scrollable).first);
    final position = scrollable.controller!.position;
    final beforeTapOffset = position.pixels;

    await tester.tap(find.byKey(const ValueKey('overview-all')));
    await tester.pumpAndSettle();

    expect(position.pixels, beforeTapOffset);
    expect(find.text('Aurora Mobile'), findsOneWidget);
    expect(find.text('ReaderKit Demo'), findsOneWidget);
  });

  testWidgets('overview status filters without scrolling from current viewport',
      (tester) async {
    await pumpApp(tester);

    final scrollable = tester.widget<Scrollable>(find.byType(Scrollable).first);
    final position = scrollable.controller!.position;
    await tester.drag(
      find.byKey(const ValueKey('home-scroll-view')),
      const Offset(0, -96),
    );
    await tester.pumpAndSettle();

    for (final key in [
      const ValueKey('overview-updates'),
      const ValueKey('overview-installing'),
      const ValueKey('overview-installed'),
    ]) {
      final beforeTapOffset = position.pixels;
      await tester.tap(find.byKey(key));
      await tester.pumpAndSettle();
      expect(position.pixels, beforeTapOffset);
      final beforeRestoreOffset = position.pixels;
      await tester.tap(find.byKey(key));
      await tester.pumpAndSettle();
      expect(position.pixels, beforeRestoreOffset);
    }
  });

  testWidgets('overview collapses into horizontal tabs and expands back',
      (tester) async {
    await pumpApp(tester);

    await tester.ensureVisible(find.byTooltip('收起应用总览'));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('收起应用总览'));
    await tester.pumpAndSettle();

    expect(find.byKey(const ValueKey('overview-compact-tabs')), findsOneWidget);
    expect(
        find.byKey(const ValueKey('overview-horizontal-tabs')), findsOneWidget);
    expect(find.text('收起为标签'), findsNothing);
    expect(find.text('应用总览'), findsNothing);
    final selectedCompactAllMetric = tester.widget<Material>(
      find
          .descendant(
            of: find.byKey(const ValueKey('overview-all')),
            matching: find.byType(Material),
          )
          .first,
    );
    expect(selectedCompactAllMetric.color, const Color(0xFF2478FF));

    await tester
        .ensureVisible(find.byKey(const ValueKey('overview-compact-tabs')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('overview-updates')));
    await tester.pumpAndSettle();
    expect(find.text('DataFlow'), findsOneWidget);
    expect(find.text('Insight Pro'), findsOneWidget);
    expect(find.text('Aurora Mobile'), findsNothing);

    final selectedCompactUpdatesMetric = tester.widget<Material>(
      find
          .descendant(
            of: find.byKey(const ValueKey('overview-updates')),
            matching: find.byType(Material),
          )
          .first,
    );
    expect(selectedCompactUpdatesMetric.color, const Color(0xFF2478FF));
    final unselectedCompactAllMetric = tester.widget<Material>(
      find
          .descendant(
            of: find.byKey(const ValueKey('overview-all')),
            matching: find.byType(Material),
          )
          .first,
    );
    expect(unselectedCompactAllMetric.color, const Color(0xFFF2F3F6));

    await tester.tap(find.byKey(const ValueKey('overview-all')));
    await tester.pumpAndSettle();
    expect(find.text('Aurora Mobile'), findsOneWidget);
    final restoredCompactAllMetric = tester.widget<Material>(
      find
          .descendant(
            of: find.byKey(const ValueKey('overview-all')),
            matching: find.byType(Material),
          )
          .first,
    );
    expect(restoredCompactAllMetric.color, const Color(0xFF2478FF));

    await tester.tap(find.byTooltip('展开应用总览'));
    await tester.pumpAndSettle();
    expect(find.text('应用总览'), findsOneWidget);
    expect(find.byTooltip('收起应用总览'), findsOneWidget);
    expect(find.text('收起为标签'), findsNothing);
    expect(find.byKey(const ValueKey('overview-compact-tabs')), findsNothing);
  });

  testWidgets('compact overview centers tapped tab horizontally',
      (tester) async {
    tester.view.physicalSize = const Size(390, 844);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);

    await pumpApp(tester);

    await tester.ensureVisible(find.byTooltip('收起应用总览'));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('收起应用总览'));
    await tester.pumpAndSettle();
    await tester
        .ensureVisible(find.byKey(const ValueKey('overview-compact-tabs')));
    await tester.pumpAndSettle();

    final scrollView = tester.widget<SingleChildScrollView>(
      find.byKey(const ValueKey('overview-horizontal-tabs')),
    );
    final controller = scrollView.controller!;
    expect(controller.position.maxScrollExtent, greaterThan(0));
    controller.jumpTo(controller.position.maxScrollExtent);
    await tester.pumpAndSettle();
    final beforeTapOffset = controller.offset;

    await tester.tap(find.byKey(const ValueKey('overview-installing')));
    await tester.pumpAndSettle();

    expect(controller.offset, lessThan(beforeTapOffset));
    final viewportCenter = tester
        .getRect(find.byKey(const ValueKey('overview-horizontal-tabs')))
        .center
        .dx;
    final selectedTabCenter = tester
        .getRect(find.byKey(const ValueKey('overview-installing')))
        .center
        .dx;
    expect((selectedTabCenter - viewportCenter).abs(), lessThanOrEqualTo(18));
  });

  testWidgets('primary controls respond to taps', (tester) async {
    await pumpApp(tester);

    final initialRefreshRotation = tester.widget<AnimatedRotation>(
      find.byKey(const ValueKey('refresh-rotation')),
    );
    expect(initialRefreshRotation.turns, 0);

    await tester.tap(find.byTooltip('同步构建'));
    await tester.pump();
    expect(find.text('已同步最新构建'), findsOneWidget);
    final activeRefreshRotation = tester.widget<AnimatedRotation>(
      find.byKey(const ValueKey('refresh-rotation')),
    );
    expect(activeRefreshRotation.turns, 1);

    await scrollUntilTextVisible(tester, 'Aurora Mobile');
    await tester.tap(find.byIcon(Icons.pause_rounded));
    await tester.pump();
    expect(find.byIcon(Icons.play_arrow_rounded), findsOneWidget);
    expect(find.byTooltip('继续'), findsOneWidget);

    await tapBottomTab(tester, 1);

    final navigationBar = tester.widget<BottomNavigationBar>(
      find.byType(BottomNavigationBar),
    );
    expect(navigationBar.currentIndex, 1);
    expect(find.text('当前设备'), findsOneWidget);
  });

  testWidgets('home devices and profile titles collapse on scroll',
      (tester) async {
    await pumpApp(tester);

    await expectCollapsibleTabTitle(tester, 'home');

    await tapBottomTab(tester, 1);
    await expectCollapsibleTabTitle(tester, 'devices');

    await tapBottomTab(tester, 3);
    await expectCollapsibleTabTitle(tester, 'profile');
  });

  testWidgets('home device shortcut switches to devices tab', (tester) async {
    await pumpApp(tester);

    await tester.tap(find.text('查看设备'));
    await tester.pumpAndSettle();

    final navigationBar = tester.widget<BottomNavigationBar>(
      find.byType(BottomNavigationBar),
    );
    expect(navigationBar.currentIndex, 1);
    expect(find.text('当前设备'), findsOneWidget);
    expect(find.text('设备池'), findsOneWidget);
  });

  testWidgets('account renewal notice is scoped to home device card',
      (tester) async {
    await pumpApp(tester);

    expect(find.text('Aurora Mobile 所属开发者账号 5 天后到期'), findsOneWidget);
    await tester.tap(find.text('去续费'));
    await tester.pumpAndSettle();

    final navigationBar = tester.widget<BottomNavigationBar>(
      find.byType(BottomNavigationBar),
    );
    expect(navigationBar.currentIndex, 3);
    expect(find.text('Aurora Mobile 所属开发者账号 5 天后到期'), findsNothing);
    expect(find.text('测试管理员 · 移动端测试组'), findsOneWidget);

    await tapBottomTab(tester, 2);
    expect(find.text('Aurora Mobile 所属开发者账号 5 天后到期'), findsNothing);
    expect(find.text('全部已读'), findsOneWidget);

    await tapBottomTab(tester, 0);
    expect(find.text('Aurora Mobile 所属开发者账号 5 天后到期'), findsOneWidget);
  });

  testWidgets('app list defaults to added order and supports manual reorder',
      (tester) async {
    await pumpApp(tester);

    await tester.ensureVisible(find.byKey(const ValueKey('build-row-aurora')));
    await tester.pumpAndSettle();
    expect(find.byType(ReorderableListView), findsNothing);
    expect(find.byIcon(Icons.drag_handle_rounded), findsNothing);
    expect(find.byKey(const ValueKey('build-sort-entry')), findsOneWidget);
    expect(
      tester.getTopLeft(find.byKey(const ValueKey('build-row-aurora'))).dy,
      lessThan(tester
          .getTopLeft(find.byKey(const ValueKey('build-row-dataflow')))
          .dy),
    );

    await tester.ensureVisible(find.byKey(const ValueKey('build-sort-entry')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('build-sort-entry')));
    await tester.pumpAndSettle();

    expect(find.byKey(const ValueKey('build-sort-sheet')), findsOneWidget);
    expect(find.byType(ReorderableListView), findsOneWidget);
    final sortList = tester.widget<ReorderableListView>(
      find.byKey(const ValueKey('build-sort-list')),
    );
    expect(sortList.proxyDecorator, isNotNull);
    expect(
      find.descendant(
        of: find.byKey(const ValueKey('build-sort-sheet')),
        matching: find.byIcon(Icons.drag_handle_rounded),
      ),
      findsWidgets,
    );
    expect(
      find.descendant(
        of: find.byKey(const ValueKey('sort-build-row-aurora')),
        matching: find.text('Aurora Mobile'),
      ),
      findsOneWidget,
    );
    expect(
      find.descendant(
        of: find.byKey(const ValueKey('sort-build-row-aurora')),
        matching: find.text('2.3.0 (23045)'),
      ),
      findsNothing,
    );
    final sortCard = tester.widget<Container>(
      find.byKey(const ValueKey('sort-build-card-aurora')),
    );
    expect(sortCard.margin, const EdgeInsets.only(bottom: 8));

    final dragGesture = await tester.startGesture(
      tester.getCenter(find.byKey(const ValueKey('sort-drag-handle-aurora'))),
    );
    await tester.pump(const Duration(milliseconds: 120));
    await dragGesture.moveBy(const Offset(0, 120));
    await tester.pump(const Duration(milliseconds: 120));
    await dragGesture.moveBy(const Offset(0, 120));
    await tester.pump(const Duration(milliseconds: 120));
    await dragGesture.up();
    await tester.pumpAndSettle();

    expect(
      tester
          .getTopLeft(find.byKey(const ValueKey('sort-build-row-dataflow')))
          .dy,
      lessThan(tester
          .getTopLeft(find.byKey(const ValueKey('sort-build-row-aurora')))
          .dy),
    );
    expect(find.text('应用排序已更新'), findsOneWidget);

    await tester.tap(find.byTooltip('关闭排序'));
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('build-sort-sheet')), findsNothing);
    expect(
      tester.getTopLeft(find.byKey(const ValueKey('build-row-dataflow'))).dy,
      lessThan(
          tester.getTopLeft(find.byKey(const ValueKey('build-row-aurora'))).dy),
    );
  });

  testWidgets('bottom tabs render all internal tool pages', (tester) async {
    await pumpApp(tester);

    final navigationBar = tester.widget<BottomNavigationBar>(
      find.byType(BottomNavigationBar),
    );
    expect(navigationBar.items.map((item) => item.label), [
      '首页',
      '设备',
      '通知',
      '我的',
    ]);
    expect(find.text('应用总览'), findsOneWidget);
    expect(find.text('DataFlow'), findsOneWidget);

    await tapBottomTab(tester, 1);
    expect(find.text('当前设备'), findsOneWidget);
    await scrollUntilTextVisible(tester, '设备池');
    expect(find.text('设备池'), findsOneWidget);
    expect(find.text('开发-iPhone-15'), findsOneWidget);
    await scrollUntilTextVisible(tester, '证书与权限');
    expect(find.text('证书与权限'), findsOneWidget);

    await tapBottomTab(tester, 2);
    expect(find.text('全部已读'), findsOneWidget);
    expect(find.text('DataFlow 新构建已上传'), findsOneWidget);
    expect(find.text('开发者账号即将到期'), findsOneWidget);

    await tapBottomTab(tester, 3);
    expect(find.text('测试管理员 · 移动端测试组'), findsOneWidget);
    await scrollUntilTextVisible(tester, '工作台');
    expect(find.text('工作台'), findsOneWidget);
    await scrollUntilTextVisible(tester, '偏好');
    expect(find.text('偏好'), findsOneWidget);
  });

  testWidgets('notifications filter notices by type', (tester) async {
    await pumpApp(tester);

    await tapBottomTab(tester, 2);
    expect(
        find.byKey(const ValueKey('notification-type-filter')), findsOneWidget);
    expect(
        find.byKey(const ValueKey('notification-filter-all')), findsOneWidget);
    expect(find.byKey(const ValueKey('notification-filter-build')),
        findsOneWidget);
    expect(find.byKey(const ValueKey('notification-filter-account')),
        findsOneWidget);
    expect(find.byKey(const ValueKey('notification-filter-device')),
        findsOneWidget);
    expect(find.text('DataFlow 新构建已上传'), findsOneWidget);
    expect(find.text('开发者账号即将到期'), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('notification-filter-account')));
    await tester.pumpAndSettle();
    expect(find.text('开发者账号即将到期'), findsOneWidget);
    expect(find.text('DataFlow 新构建已上传'), findsNothing);
    expect(find.text('Ops-iPhone-14 已加入设备池'), findsNothing);

    await tester.tap(find.byKey(const ValueKey('notification-filter-device')));
    await tester.pumpAndSettle();
    expect(find.text('Ops-iPhone-14 已加入设备池'), findsOneWidget);
    expect(find.text('开发者账号即将到期'), findsNothing);

    await tester.tap(find.byKey(const ValueKey('notification-filter-build')));
    await tester.pumpAndSettle();
    expect(find.text('DataFlow 新构建已上传'), findsOneWidget);
    await scrollUntilTextVisible(tester, 'API Explorer 崩溃日志已收集');
    expect(find.text('API Explorer 崩溃日志已收集'), findsOneWidget);
    expect(find.text('Ops-iPhone-14 已加入设备池'), findsNothing);

    await tester
        .ensureVisible(find.byKey(const ValueKey('notification-filter-all')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('notification-filter-all')));
    await tester.pumpAndSettle();
    expect(find.text('DataFlow 新构建已上传'), findsOneWidget);
    expect(find.text('开发者账号即将到期'), findsOneWidget);
  });

  testWidgets('notifications pin filter and collapse title on scroll',
      (tester) async {
    await pumpApp(tester);

    await tapBottomTab(tester, 2);
    final titleBar = tester.widget<SliverAppBar>(
      find.byKey(const ValueKey('notification-title-bar')),
    );
    expect(titleBar.pinned, isTrue);
    expect(titleBar.centerTitle, isTrue);
    expect(titleBar.expandedHeight, 142);
    expect(
        find.byKey(const ValueKey('notification-small-title')), findsNothing);
    final initialLargeTitle = tester.widget<Opacity>(
      find.byKey(const ValueKey('notification-large-title-opacity')),
    );
    expect(initialLargeTitle.opacity, 1);

    final beforeFilterTop = tester
        .getTopLeft(find.byKey(const ValueKey('notification-type-filter')))
        .dy;
    final beforeLargeTitleBottom = tester
        .getBottomLeft(find.byKey(const ValueKey('notification-large-title')))
        .dy;
    expect(beforeLargeTitleBottom, lessThan(beforeFilterTop));
    expect(
      tester
          .getTopLeft(find.byKey(const ValueKey('notification-large-title')))
          .dx,
      closeTo(16, 1),
    );

    await tester.drag(
      find.byKey(const ValueKey('notification-scroll-view')),
      const Offset(0, -32),
    );
    await tester.pumpAndSettle();
    final midLargeTitle = tester.widget<Opacity>(
      find.byKey(const ValueKey('notification-large-title-opacity')),
    );
    final midSmallTitle = tester.widget<Opacity>(
      find.byKey(const ValueKey('notification-small-title-opacity')),
    );
    final midSmallTitleFlip = tester.widget<Transform>(
      find.byKey(const ValueKey('notification-small-title-flip')),
    );
    expect(midLargeTitle.opacity, allOf(greaterThan(0), lessThanOrEqualTo(.5)));
    expect(midSmallTitle.opacity, allOf(greaterThan(0), lessThan(1)));
    expect(midSmallTitleFlip.alignment, Alignment.bottomCenter);
    expect(midSmallTitleFlip.transform.storage[5], lessThan(.98));

    await tester.drag(
      find.byKey(const ValueKey('notification-scroll-view')),
      const Offset(0, -360),
    );
    await tester.pumpAndSettle();

    expect(
        find.byKey(const ValueKey('notification-type-filter')), findsOneWidget);
    final afterFilterTop = tester
        .getTopLeft(find.byKey(const ValueKey('notification-type-filter')))
        .dy;
    expect(afterFilterTop, lessThan(beforeFilterTop));
    expect(afterFilterTop, lessThanOrEqualTo(60));
    expect(
        find.byKey(const ValueKey('notification-small-title')), findsOneWidget);
    expect(
        find.byKey(const ValueKey('notification-large-title')), findsNothing);
  });

  testWidgets('home app list opens app details and build details',
      (tester) async {
    await pumpApp(tester);

    final initialOverviewScanRotation = tester.widget<AnimatedRotation>(
      find.byKey(const ValueKey('overview-scan-rotation')),
    );
    expect(initialOverviewScanRotation.turns, 0);

    await tester.tap(find.byTooltip('扫描新版本'));
    await tester.pump();
    expect(find.text('已扫描最新构建'), findsOneWidget);
    final activeOverviewScanRotation = tester.widget<AnimatedRotation>(
      find.byKey(const ValueKey('overview-scan-rotation')),
    );
    expect(activeOverviewScanRotation.turns, 1);

    await scrollUntilTextVisible(tester, 'DataFlow');
    await tester.tap(find.text('DataFlow'));
    await tester.pumpAndSettle();
    expect(find.text('应用详情'), findsOneWidget);
    expect(find.text('应用信息'), findsOneWidget);
    expect(find.text('默认环境'), findsOneWidget);
    expect(find.text('环境分类'), findsNothing);
    expect(find.text('负责人'), findsNothing);
    expect(find.text('负责人应用'), findsNothing);
    expect(find.text('最近构建'), findsWidgets);
    expect(find.text('修复数据同步异常问题'), findsNothing);

    await scrollUntilTextVisible(tester, '1.8.4 (18423)');
    await tester.tap(find.text('1.8.4 (18423)'));
    await tester.pumpAndSettle();
    expect(find.text('构建详情'), findsOneWidget);
    expect(find.text('DataFlow'), findsOneWidget);
    expect(find.text('修复数据同步异常问题'), findsOneWidget);
  });

  testWidgets('home build row opens app details before build details',
      (tester) async {
    await pumpApp(tester);

    await scrollUntilTextVisible(tester, 'Aurora Mobile');
    await tester.tap(find.text('Aurora Mobile'));
    await tester.pumpAndSettle();

    expect(find.text('应用详情'), findsOneWidget);
    expect(find.text('应用信息'), findsOneWidget);
    expect(find.text('负责人'), findsNothing);
    expect(find.text('负责人应用'), findsNothing);
    await scrollUntilTextVisible(tester, '测试状态');
    expect(find.text('测试状态'), findsOneWidget);
    expect(find.text('构建详情'), findsNothing);

    await scrollUntilTextVisible(tester, '2.3.0 (23045)');
    await tester.tap(find.text('2.3.0 (23045)'));
    await tester.pumpAndSettle();
    expect(find.text('构建详情'), findsOneWidget);
    expect(find.text('Aurora Mobile'), findsOneWidget);
    expect(find.text('登录链路开发环境'), findsOneWidget);
    expect(find.text('构建信息'), findsOneWidget);
  });
}
