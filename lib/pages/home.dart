import 'package:flutter/material.dart';
import 'package:testflying/models/internal_build.dart';
import 'package:testflying/models/workspace_data.dart';
import 'package:testflying/pages/app_details.dart';
import 'package:testflying/pages/tab_pages.dart';
import 'package:testflying/services/install_launcher.dart';
import 'package:testflying/services/mock_testflight_service.dart';
import 'package:testflying/services/testflight_service.dart';

enum _OverviewTarget {
  all,
  updates,
  installing,
  installed,
}

class HomePage extends StatefulWidget {
  const HomePage({super.key, this.service});

  final TestFlightService? service;

  @override
  State<HomePage> createState() => _HomePageState();
}

class _HomePageState extends State<HomePage> {
  static const _blue = Color(0xFF2478FF);
  static const _textPrimary = Color(0xFF10131A);
  static const _textSecondary = Color(0xFF7B8190);
  static const _line = Color(0xFFE7E9EF);
  static const _overviewSurface = Color(0xFFF2F3F6);
  static const _overviewSurfaceBorder = Color(0xFFD7DBE4);
  static const _overviewSelectedBorder = Color(0xFF1A64D6);
  static const _overviewLabel = Color(0xFF6B7280);
  static const _overviewToggle = Color(0xFF9AA1AD);

  _OverviewTarget? _selectedOverviewTarget;
  bool _isOverviewCollapsed = false;
  bool _isLoadingWorkspace = true;
  Object? _workspaceError;
  int _navIndex = 0;
  late final TestFlightService _service =
      widget.service ?? MockTestFlightService();
  TestFlightWorkspace? _workspace;

  TestFlightWorkspace get _activeWorkspace => _workspace!;

  List<InternalBuild> get _builds => _activeWorkspace.builds;

  List<InternalBuild> get _visibleBuilds {
    switch (_selectedOverviewTarget) {
      case null:
      case _OverviewTarget.all:
        return _builds;
      case _OverviewTarget.updates:
        return _builds
            .where((build) => build.status == BuildStatus.updateAvailable)
            .toList();
      case _OverviewTarget.installing:
        return _builds
            .where((build) => build.status == BuildStatus.installing)
            .toList();
      case _OverviewTarget.installed:
        return _builds
            .where((build) => build.status == BuildStatus.installed)
            .toList();
    }
  }

  @override
  void initState() {
    super.initState();
    _loadInitialWorkspace();
  }

  Future<void> _loadInitialWorkspace() async {
    setState(() {
      _isLoadingWorkspace = true;
      _workspaceError = null;
    });

    try {
      final workspace = await _service.loadWorkspace();
      if (!mounted) {
        return;
      }
      setState(() {
        _workspace = workspace;
        _isLoadingWorkspace = false;
      });
    } catch (error) {
      if (!mounted) {
        return;
      }
      setState(() {
        _workspaceError = error;
        _isLoadingWorkspace = false;
      });
    }
  }

  Future<void> _reloadWorkspace(String successMessage) async {
    try {
      final workspace = await _service.loadWorkspace();
      if (!mounted) {
        return;
      }
      setState(() => _workspace = workspace);
      _showStatusMessage(successMessage);
    } catch (_) {
      if (_workspace == null) {
        await _loadInitialWorkspace();
        return;
      }
      _showStatusMessage('同步失败，请稍后重试');
    }
  }

  Future<void> _toggleInstallState(InternalBuild build) async {
    final message = build.status == BuildStatus.installing
        ? (build.isPaused ? '继续安装 ${build.name}' : '已暂停 ${build.name}')
        : '已打开 ${build.installInfo.platform.label} 安装入口：${build.name}';

    try {
      final workspace = await _service.toggleInstallState(
        _activeWorkspace,
        build,
      );
      if (!mounted) {
        return;
      }
      setState(() => _workspace = workspace);
      _showStatusMessage(message);
    } on InstallLaunchException catch (error) {
      _showStatusMessage(error.message);
    } catch (_) {
      _showStatusMessage('安装状态保存失败');
    }
  }

  void _showStatusMessage(String message) {
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(content: Text(message), duration: const Duration(seconds: 1)),
    );
  }

  void _openAppDetails(InternalBuild build) {
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (context) => AppDetailsPage(
          app: build,
          onInstall: _toggleInstallState,
        ),
      ),
    );
  }

  void _selectOverviewTarget(_OverviewTarget target) {
    final nextTarget =
        target == _OverviewTarget.all || _selectedOverviewTarget == target
            ? null
            : target;

    setState(() => _selectedOverviewTarget = nextTarget);
  }

  Future<void> _reorderVisibleBuilds(int oldIndex, int newIndex) async {
    final visibleBuilds = _visibleBuilds;
    final beforeOrder = _builds.map((build) => build.id).join(',');

    try {
      final workspace = await _service.reorderVisibleBuilds(
        _activeWorkspace,
        visibleBuilds,
        oldIndex,
        newIndex,
      );
      if (!mounted) {
        return;
      }
      setState(() => _workspace = workspace);
    } catch (_) {
      _showStatusMessage('应用排序保存失败');
      return;
    }

    final afterOrder = _builds.map((build) => build.id).join(',');
    if (beforeOrder != afterOrder) {
      _showStatusMessage('应用排序已更新');
    }
  }

  void _openSortSheet() {
    showModalBottomSheet<void>(
      context: context,
      backgroundColor: Colors.transparent,
      isScrollControlled: true,
      builder: (context) => _BuildSortSheet(
        builds: _visibleBuilds,
        onReorder: _reorderVisibleBuilds,
      ),
    );
  }

  Widget _buildSecondaryTab(TestFlightWorkspace workspace) {
    switch (_navIndex) {
      case 1:
        return DevicesPage(
          currentDevice: workspace.currentDevice,
          devices:
              workspace.devices.where((device) => !device.isCurrent).toList(),
          developerAccount: workspace.renewalAccount,
          onMessage: _showStatusMessage,
        );
      case 2:
        return NotificationsPage(
          notifications: workspace.notifications,
          onMessage: _showStatusMessage,
        );
      case 3:
        return ProfilePage(
          profile: workspace.profile,
          onMessage: _showStatusMessage,
        );
      default:
        return const SizedBox.shrink();
    }
  }

  @override
  Widget build(BuildContext context) {
    final workspace = _workspace;

    return Scaffold(
      backgroundColor: const Color(0xFFF5F6FA),
      bottomNavigationBar: _BottomNav(
        selectedIndex: _navIndex,
        onChanged: (index) => setState(() => _navIndex = index),
      ),
      body: SafeArea(
        child: _buildBody(workspace),
      ),
    );
  }

  Widget _buildBody(TestFlightWorkspace? workspace) {
    if (_isLoadingWorkspace) {
      return const _WorkspaceStateView(
        key: ValueKey('workspace-loading'),
        icon: Icons.sync_rounded,
        title: '正在加载测试工作台',
        subtitle: '同步本机安装任务、应用排序和测试数据',
        isLoading: true,
      );
    }

    if (_workspaceError != null || workspace == null) {
      return _WorkspaceStateView(
        key: const ValueKey('workspace-error'),
        icon: Icons.cloud_off_rounded,
        title: '工作台加载失败',
        subtitle: '本地状态或测试数据暂时不可用，可以重试加载',
        actionLabel: '重试',
        onAction: _loadInitialWorkspace,
      );
    }

    if (_navIndex != 0) {
      return _buildSecondaryTab(workspace);
    }

    final visibleBuilds = _visibleBuilds;

    return CollapsibleTabScrollView(
      title: '内测工具',
      titleKeyPrefix: 'home',
      trailing: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          _RefreshIconAction(
            tooltip: '同步构建',
            icon: Icons.refresh_rounded,
            rotationKey: const ValueKey('refresh-rotation'),
            onPressed: () => _reloadWorkspace('已同步最新构建'),
          ),
          const SizedBox(width: 6),
          _IconAction(
            icon: Icons.settings_rounded,
            tooltip: '设置',
            onPressed: () => _showStatusMessage('设备与证书设置'),
          ),
        ],
      ),
      children: [
        _DeviceShortcut(
          currentDevice: workspace.currentDevice,
          developerAccount: workspace.renewalAccount,
          showRenewalReminder: workspace.renewalAccount != null,
          onOpen: () => setState(() => _navIndex = 1),
          onRenew: () => setState(() => _navIndex = 3),
        ),
        const SizedBox(height: 18),
        _AppOverview(
          builds: _builds,
          selectedTarget: _selectedOverviewTarget,
          collapsed: _isOverviewCollapsed,
          onScan: () => _reloadWorkspace('已扫描最新构建'),
          onSelect: _selectOverviewTarget,
          onToggleCollapsed: () => setState(
            () => _isOverviewCollapsed = !_isOverviewCollapsed,
          ),
        ),
        const SizedBox(height: 18),
        _SectionTitle(
          title: '应用列表',
          count: visibleBuilds.length,
        ),
        const SizedBox(height: 10),
        if (visibleBuilds.isEmpty)
          _BuildEmptyState(
            filtered: _selectedOverviewTarget != null,
          )
        else
          _BuildGroup(
            builds: visibleBuilds,
            onAction: _toggleInstallState,
            onOpen: _openAppDetails,
            onSort: _openSortSheet,
          ),
        const SizedBox(height: 20),
      ],
    );
  }
}

class _RefreshIconAction extends StatefulWidget {
  const _RefreshIconAction({
    required this.tooltip,
    required this.icon,
    required this.rotationKey,
    required this.onPressed,
  });

  final String tooltip;
  final IconData icon;
  final Key rotationKey;
  final VoidCallback onPressed;

  @override
  State<_RefreshIconAction> createState() => _RefreshIconActionState();
}

class _RefreshIconActionState extends State<_RefreshIconAction> {
  double _turns = 0;

  void _handlePressed() {
    setState(() => _turns += 1);
    widget.onPressed();
  }

  @override
  Widget build(BuildContext context) {
    return IconButton(
      tooltip: widget.tooltip,
      onPressed: _handlePressed,
      icon: AnimatedRotation(
        key: widget.rotationKey,
        turns: _turns,
        duration: const Duration(milliseconds: 520),
        curve: Curves.easeOutCubic,
        child: Icon(
          widget.icon,
          color: _HomePageState._blue,
          size: 30,
        ),
      ),
    );
  }
}

class _WorkspaceStateView extends StatelessWidget {
  const _WorkspaceStateView({
    super.key,
    required this.icon,
    required this.title,
    required this.subtitle,
    this.isLoading = false,
    this.actionLabel,
    this.onAction,
  });

  final IconData icon;
  final String title;
  final String subtitle;
  final bool isLoading;
  final String? actionLabel;
  final VoidCallback? onAction;

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(24),
        child: Container(
          width: double.infinity,
          padding: const EdgeInsets.all(22),
          decoration: BoxDecoration(
            color: Colors.white,
            border: Border.all(color: _HomePageState._line),
            borderRadius: BorderRadius.circular(18),
          ),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              SizedBox.square(
                dimension: 54,
                child: isLoading
                    ? const CircularProgressIndicator(
                        key: ValueKey('workspace-loading-indicator'),
                        strokeWidth: 3.2,
                      )
                    : DecoratedBox(
                        decoration: BoxDecoration(
                          color: const Color(0xFFE8F1FF),
                          borderRadius: BorderRadius.circular(16),
                        ),
                        child: Icon(
                          icon,
                          color: _HomePageState._blue,
                          size: 30,
                        ),
                      ),
              ),
              const SizedBox(height: 16),
              Text(
                title,
                textAlign: TextAlign.center,
                style: const TextStyle(
                  color: _HomePageState._textPrimary,
                  fontSize: 19,
                  fontWeight: FontWeight.w800,
                ),
              ),
              const SizedBox(height: 7),
              Text(
                subtitle,
                textAlign: TextAlign.center,
                style: const TextStyle(
                  color: _HomePageState._textSecondary,
                  fontSize: 14,
                  fontWeight: FontWeight.w600,
                  height: 1.35,
                ),
              ),
              if (actionLabel != null && onAction != null) ...[
                const SizedBox(height: 18),
                FilledButton(
                  key: const ValueKey('workspace-retry'),
                  onPressed: onAction,
                  child: Text(actionLabel!),
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

class _IconAction extends StatelessWidget {
  const _IconAction({
    required this.icon,
    required this.tooltip,
    required this.onPressed,
  });

  final IconData icon;
  final String tooltip;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    return IconButton(
      icon: Icon(icon, color: _HomePageState._blue, size: 30),
      tooltip: tooltip,
      onPressed: onPressed,
    );
  }
}

class _DeviceShortcut extends StatelessWidget {
  const _DeviceShortcut({
    required this.currentDevice,
    required this.developerAccount,
    required this.showRenewalReminder,
    required this.onOpen,
    required this.onRenew,
  });

  final TestDevice currentDevice;
  final DeveloperAccount? developerAccount;
  final bool showRenewalReminder;
  final VoidCallback onOpen;
  final VoidCallback onRenew;

  @override
  Widget build(BuildContext context) {
    return Material(
      color: Colors.white,
      borderRadius: BorderRadius.circular(16),
      child: Container(
        padding: const EdgeInsets.all(16),
        decoration: BoxDecoration(
          border: Border.all(color: _HomePageState._line),
          borderRadius: BorderRadius.circular(16),
        ),
        child: Column(
          children: [
            InkWell(
              onTap: onOpen,
              borderRadius: BorderRadius.circular(12),
              child: Padding(
                padding: const EdgeInsets.symmetric(vertical: 2),
                child: Row(
                  children: [
                    const Icon(
                      Icons.phone_iphone_rounded,
                      color: _HomePageState._blue,
                      size: 30,
                    ),
                    const SizedBox(width: 12),
                    Expanded(
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          const Text(
                            '设备与证书',
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: TextStyle(
                              color: _HomePageState._textPrimary,
                              fontSize: 17,
                              fontWeight: FontWeight.w800,
                            ),
                          ),
                          const SizedBox(height: 4),
                          Text(
                            '${currentDevice.name} ${currentDevice.status} · ${developerAccount == null ? '暂无账号续费提醒' : '开发者账号续费待处理'}',
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: const TextStyle(
                              color: _HomePageState._textSecondary,
                              fontSize: 13,
                              fontWeight: FontWeight.w600,
                            ),
                          ),
                        ],
                      ),
                    ),
                    const SizedBox(width: 8),
                    const Text(
                      '查看设备',
                      style: TextStyle(
                        color: _HomePageState._blue,
                        fontSize: 14,
                        fontWeight: FontWeight.w800,
                      ),
                    ),
                    const SizedBox(width: 2),
                    const Icon(
                      Icons.chevron_right_rounded,
                      color: _HomePageState._blue,
                    ),
                  ],
                ),
              ),
            ),
            if (showRenewalReminder && developerAccount != null) ...[
              const SizedBox(height: 12),
              _DeviceRenewalNotice(
                developerAccount: developerAccount!,
                onRenew: onRenew,
              ),
            ],
          ],
        ),
      ),
    );
  }
}

class _DeviceRenewalNotice extends StatelessWidget {
  const _DeviceRenewalNotice({
    required this.developerAccount,
    required this.onRenew,
  });

  final DeveloperAccount developerAccount;
  final VoidCallback onRenew;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.fromLTRB(10, 10, 8, 10),
      decoration: BoxDecoration(
        color: const Color(0xFFFEF2F2),
        border: Border.all(color: const Color(0xFFF87171), width: 1.2),
        borderRadius: BorderRadius.circular(12),
      ),
      child: Row(
        children: [
          const Icon(
            Icons.warning_amber_rounded,
            color: Color(0xFFDC2626),
            size: 24,
          ),
          const SizedBox(width: 8),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                const Text(
                  '账号续费预警',
                  style: TextStyle(
                    color: Color(0xFF991B1B),
                    fontSize: 12,
                    fontWeight: FontWeight.w900,
                    height: 1.1,
                  ),
                ),
                const SizedBox(height: 3),
                Text(
                  developerAccount.renewalTitle,
                  maxLines: 2,
                  overflow: TextOverflow.visible,
                  softWrap: true,
                  style: const TextStyle(
                    color: Color(0xFFB91C1C),
                    fontSize: 13,
                    fontWeight: FontWeight.w800,
                    height: 1.2,
                  ),
                ),
              ],
            ),
          ),
          const SizedBox(width: 8),
          TextButton(
            onPressed: onRenew,
            style: TextButton.styleFrom(
              foregroundColor: Colors.white,
              backgroundColor: const Color(0xFFDC2626),
              minimumSize: const Size(62, 34),
              padding: const EdgeInsets.symmetric(horizontal: 8),
              textStyle: const TextStyle(
                fontSize: 13,
                fontWeight: FontWeight.w800,
              ),
            ),
            child: Text(developerAccount.renewalActionLabel),
          ),
        ],
      ),
    );
  }
}

class _AppOverview extends StatelessWidget {
  const _AppOverview({
    required this.builds,
    required this.selectedTarget,
    required this.collapsed,
    required this.onScan,
    required this.onSelect,
    required this.onToggleCollapsed,
  });

  final List<InternalBuild> builds;
  final _OverviewTarget? selectedTarget;
  final bool collapsed;
  final VoidCallback onScan;
  final ValueChanged<_OverviewTarget> onSelect;
  final VoidCallback onToggleCollapsed;

  @override
  Widget build(BuildContext context) {
    final installing =
        builds.where((build) => build.status == BuildStatus.installing).length;
    final updates = builds
        .where((build) => build.status == BuildStatus.updateAvailable)
        .length;
    final installed =
        builds.where((build) => build.status == BuildStatus.installed).length;
    final metrics = [
      _OverviewMetricData(
        key: const ValueKey('overview-all'),
        target: _OverviewTarget.all,
        label: '全部应用',
        value: '${builds.length}',
        icon: Icons.apps_rounded,
        selected: false,
      ),
      _OverviewMetricData(
        key: const ValueKey('overview-updates'),
        target: _OverviewTarget.updates,
        label: '待更新',
        value: '$updates',
        icon: Icons.system_update_alt_rounded,
        selected: selectedTarget == _OverviewTarget.updates,
      ),
      _OverviewMetricData(
        key: const ValueKey('overview-installing'),
        target: _OverviewTarget.installing,
        label: '安装中',
        value: '$installing',
        icon: Icons.downloading_rounded,
        selected: selectedTarget == _OverviewTarget.installing,
      ),
      _OverviewMetricData(
        key: const ValueKey('overview-installed'),
        target: _OverviewTarget.installed,
        label: '已安装',
        value: '$installed',
        icon: Icons.check_circle_rounded,
        selected: selectedTarget == _OverviewTarget.installed,
      ),
    ];

    if (collapsed) {
      return _CompactOverviewTabs(
        metrics: metrics,
        onScan: onScan,
        onSelect: onSelect,
        onExpand: onToggleCollapsed,
      );
    }

    return Container(
      padding: const EdgeInsets.fromLTRB(16, 16, 16, 10),
      decoration: BoxDecoration(
        color: Colors.white,
        border: Border.all(color: _HomePageState._line),
        borderRadius: BorderRadius.circular(16),
      ),
      child: Column(
        children: [
          Row(
            children: [
              const Expanded(
                child: Text(
                  '应用总览',
                  style: TextStyle(
                    color: _HomePageState._textPrimary,
                    fontSize: 18,
                    fontWeight: FontWeight.w800,
                  ),
                ),
              ),
              _RefreshIconAction(
                tooltip: '扫描新版本',
                icon: Icons.sync_rounded,
                rotationKey: const ValueKey('overview-scan-rotation'),
                onPressed: onScan,
              ),
            ],
          ),
          const SizedBox(height: 8),
          GridView.count(
            crossAxisCount: 2,
            childAspectRatio: 3,
            shrinkWrap: true,
            physics: const NeverScrollableScrollPhysics(),
            crossAxisSpacing: 10,
            mainAxisSpacing: 10,
            children: metrics
                .map(
                  (metric) => _OverviewMetric(
                    key: metric.key,
                    label: metric.label,
                    value: metric.value,
                    icon: metric.icon,
                    selected: metric.selected,
                    onTap: () => onSelect(metric.target),
                  ),
                )
                .toList(),
          ),
          const SizedBox(height: 2),
          Center(
            child: _OverviewToggleArrow(
              tooltip: '收起应用总览',
              icon: Icons.keyboard_arrow_up_rounded,
              padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 2),
              onTap: onToggleCollapsed,
            ),
          ),
        ],
      ),
    );
  }
}

class _OverviewMetricData {
  const _OverviewMetricData({
    required this.key,
    required this.target,
    required this.label,
    required this.value,
    required this.icon,
    required this.selected,
  });

  final Key key;
  final _OverviewTarget target;
  final String label;
  final String value;
  final IconData icon;
  final bool selected;
}

class _CompactOverviewTabs extends StatefulWidget {
  const _CompactOverviewTabs({
    required this.metrics,
    required this.onScan,
    required this.onSelect,
    required this.onExpand,
  });

  final List<_OverviewMetricData> metrics;
  final VoidCallback onScan;
  final ValueChanged<_OverviewTarget> onSelect;
  final VoidCallback onExpand;

  @override
  State<_CompactOverviewTabs> createState() => _CompactOverviewTabsState();
}

class _CompactOverviewTabsState extends State<_CompactOverviewTabs> {
  final _scrollController = ScrollController();
  final _scrollViewportKey = GlobalKey();
  final _tabKeys = <_OverviewTarget, GlobalKey>{
    _OverviewTarget.all: GlobalKey(),
    _OverviewTarget.updates: GlobalKey(),
    _OverviewTarget.installing: GlobalKey(),
    _OverviewTarget.installed: GlobalKey(),
  };

  @override
  void dispose() {
    _scrollController.dispose();
    super.dispose();
  }

  void _handleSelect(_OverviewMetricData metric) {
    widget.onSelect(metric.target);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) {
        return;
      }
      _centerTab(metric.target);
    });
  }

  void _centerTab(_OverviewTarget target) {
    final tabContext = _tabKeys[target]?.currentContext;
    final viewportContext = _scrollViewportKey.currentContext;
    if (tabContext == null ||
        viewportContext == null ||
        !_scrollController.hasClients) {
      return;
    }

    final tabBox = tabContext.findRenderObject() as RenderBox?;
    final viewportBox = viewportContext.findRenderObject() as RenderBox?;
    if (tabBox == null || viewportBox == null || !tabBox.attached) {
      return;
    }

    final tabLeft = tabBox
        .localToGlobal(
          Offset.zero,
          ancestor: viewportBox,
        )
        .dx;
    final tabCenter = tabLeft + tabBox.size.width / 2;
    final targetOffset =
        _scrollController.offset + tabCenter - viewportBox.size.width / 2;
    final position = _scrollController.position;
    final clampedOffset = targetOffset.clamp(
      position.minScrollExtent,
      position.maxScrollExtent,
    );

    _scrollController.animateTo(
      clampedOffset.toDouble(),
      duration: const Duration(milliseconds: 220),
      curve: Curves.easeOutCubic,
    );
  }

  @override
  Widget build(BuildContext context) {
    final hasActiveFilter = widget.metrics.any(
      (metric) => metric.target != _OverviewTarget.all && metric.selected,
    );

    return Container(
      key: const ValueKey('overview-compact-tabs'),
      padding: const EdgeInsets.fromLTRB(8, 8, 8, 8),
      decoration: BoxDecoration(
        color: Colors.white,
        border: Border.all(color: _HomePageState._line),
        borderRadius: BorderRadius.circular(16),
      ),
      child: Row(
        children: [
          _OverviewToggleArrow(
            tooltip: '展开应用总览',
            icon: Icons.keyboard_arrow_down_rounded,
            onTap: widget.onExpand,
          ),
          Expanded(
            child: SizedBox(
              key: _scrollViewportKey,
              child: SingleChildScrollView(
                key: const ValueKey('overview-horizontal-tabs'),
                controller: _scrollController,
                scrollDirection: Axis.horizontal,
                child: Row(
                  children: [
                    for (final metric in widget.metrics) ...[
                      Container(
                        key: _tabKeys[metric.target],
                        child: _CompactOverviewTab(
                          key: metric.key,
                          label: metric.label,
                          value: metric.value,
                          selected: metric.target == _OverviewTarget.all
                              ? !hasActiveFilter
                              : metric.selected,
                          onTap: () => _handleSelect(metric),
                        ),
                      ),
                      const SizedBox(width: 8),
                    ],
                  ],
                ),
              ),
            ),
          ),
          _RefreshIconAction(
            tooltip: '扫描新版本',
            icon: Icons.sync_rounded,
            rotationKey: const ValueKey('overview-scan-rotation'),
            onPressed: widget.onScan,
          ),
        ],
      ),
    );
  }
}

class _OverviewToggleArrow extends StatelessWidget {
  const _OverviewToggleArrow({
    required this.tooltip,
    required this.icon,
    required this.onTap,
    this.padding = const EdgeInsets.all(7),
  });

  final String tooltip;
  final IconData icon;
  final VoidCallback onTap;
  final EdgeInsetsGeometry padding;

  @override
  Widget build(BuildContext context) {
    return Tooltip(
      message: tooltip,
      child: Semantics(
        button: true,
        label: tooltip,
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: onTap,
          child: Padding(
            padding: padding,
            child: Icon(
              icon,
              color: _HomePageState._overviewToggle,
              size: 24,
            ),
          ),
        ),
      ),
    );
  }
}

class _CompactOverviewTab extends StatelessWidget {
  const _CompactOverviewTab({
    super.key,
    required this.label,
    required this.value,
    required this.selected,
    required this.onTap,
  });

  final String label;
  final String value;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return Material(
      color: selected ? _HomePageState._blue : _HomePageState._overviewSurface,
      clipBehavior: Clip.antiAlias,
      shape: RoundedRectangleBorder(
        side: BorderSide(
          color: selected
              ? _HomePageState._overviewSelectedBorder
              : _HomePageState._overviewSurfaceBorder,
          width: selected ? 1.2 : 1,
        ),
        borderRadius: BorderRadius.circular(999),
      ),
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(999),
        child: Container(
          constraints: const BoxConstraints(minWidth: 92),
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 9),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Text(
                label,
                style: TextStyle(
                  color:
                      selected ? Colors.white : _HomePageState._overviewLabel,
                  fontSize: 13,
                  fontWeight: FontWeight.w800,
                ),
              ),
              const SizedBox(width: 8),
              Text(
                value,
                style: TextStyle(
                  color: selected ? Colors.white : _HomePageState._textPrimary,
                  fontSize: 15,
                  fontWeight: FontWeight.w900,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _OverviewMetric extends StatelessWidget {
  const _OverviewMetric({
    super.key,
    required this.label,
    required this.value,
    required this.icon,
    required this.selected,
    required this.onTap,
  });

  final String label;
  final String value;
  final IconData icon;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return Material(
      color: selected ? _HomePageState._blue : _HomePageState._overviewSurface,
      clipBehavior: Clip.antiAlias,
      shape: RoundedRectangleBorder(
        side: BorderSide(
          color: selected
              ? _HomePageState._overviewSelectedBorder
              : _HomePageState._overviewSurfaceBorder,
          width: selected ? 1.2 : 1,
        ),
        borderRadius: BorderRadius.circular(12),
      ),
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(12),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
          child: Row(
            children: [
              Icon(
                icon,
                color: selected ? Colors.white : _HomePageState._blue,
                size: 22,
              ),
              const SizedBox(width: 9),
              Expanded(
                child: Text(
                  label,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(
                    fontSize: 12,
                    fontWeight: FontWeight.w700,
                  ).copyWith(
                    color:
                        selected ? Colors.white : _HomePageState._overviewLabel,
                  ),
                ),
              ),
              const SizedBox(width: 6),
              Text(
                value,
                style: const TextStyle(
                  fontSize: 17,
                  fontWeight: FontWeight.w800,
                ).copyWith(
                  color: selected ? Colors.white : _HomePageState._textPrimary,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _SectionTitle extends StatelessWidget {
  const _SectionTitle({
    required this.title,
    required this.count,
  });

  final String title;
  final int count;

  @override
  Widget build(BuildContext context) {
    return Row(
      children: [
        Text(
          title,
          style: const TextStyle(
            color: _HomePageState._textPrimary,
            fontSize: 21,
            fontWeight: FontWeight.w800,
          ),
        ),
        const SizedBox(width: 8),
        Container(
          padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
          decoration: BoxDecoration(
            color: const Color(0xFFF2F3F6),
            borderRadius: BorderRadius.circular(10),
            border: Border.all(color: const Color(0xFFD7DBE4)),
          ),
          child: Text(
            '$count',
            style: const TextStyle(
              color: _HomePageState._textSecondary,
              fontSize: 13,
              fontWeight: FontWeight.w700,
            ),
          ),
        ),
      ],
    );
  }
}

class _BuildGroup extends StatelessWidget {
  const _BuildGroup({
    required this.builds,
    required this.onAction,
    required this.onOpen,
    required this.onSort,
  });

  final List<InternalBuild> builds;
  final ValueChanged<InternalBuild> onAction;
  final ValueChanged<InternalBuild> onOpen;
  final VoidCallback onSort;

  @override
  Widget build(BuildContext context) {
    return Container(
      decoration: BoxDecoration(
        color: Colors.white,
        border: Border.all(color: _HomePageState._line),
        borderRadius: BorderRadius.circular(16),
      ),
      child: Column(
        children: [
          for (var index = 0; index < builds.length; index++) ...[
            _BuildRow(
              key: ValueKey('build-row-${builds[index].id}'),
              item: builds[index],
              onAction: () => onAction(builds[index]),
              onOpen: () => onOpen(builds[index]),
            ),
            const Divider(height: 1, indent: 76, color: _HomePageState._line),
          ],
          _BuildSortEntry(onTap: onSort),
        ],
      ),
    );
  }
}

class _BuildSortEntry extends StatelessWidget {
  const _BuildSortEntry({required this.onTap});

  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return Material(
      color: Colors.transparent,
      child: InkWell(
        key: const ValueKey('build-sort-entry'),
        onTap: onTap,
        borderRadius: const BorderRadius.vertical(
          bottom: Radius.circular(16),
        ),
        child: const Padding(
          padding: EdgeInsets.symmetric(horizontal: 16, vertical: 13),
          child: Row(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              Icon(
                Icons.swap_vert_rounded,
                color: _HomePageState._textSecondary,
                size: 19,
              ),
              SizedBox(width: 6),
              Text(
                '调整排序',
                style: TextStyle(
                  color: _HomePageState._textSecondary,
                  fontSize: 14,
                  fontWeight: FontWeight.w800,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _BuildEmptyState extends StatelessWidget {
  const _BuildEmptyState({required this.filtered});

  final bool filtered;

  @override
  Widget build(BuildContext context) {
    return Container(
      key: const ValueKey('build-empty-state'),
      width: double.infinity,
      padding: const EdgeInsets.fromLTRB(18, 24, 18, 22),
      decoration: BoxDecoration(
        color: Colors.white,
        border: Border.all(color: _HomePageState._line),
        borderRadius: BorderRadius.circular(16),
      ),
      child: Column(
        children: [
          const Icon(
            Icons.inventory_2_outlined,
            color: _HomePageState._textSecondary,
            size: 36,
          ),
          const SizedBox(height: 10),
          Text(
            filtered ? '没有符合条件的应用' : '暂无应用构建',
            style: const TextStyle(
              color: _HomePageState._textPrimary,
              fontSize: 16,
              fontWeight: FontWeight.w800,
            ),
          ),
          const SizedBox(height: 5),
          Text(
            filtered ? '切回全部应用或等待新的包上传' : '上传新包后会自动出现在这里',
            textAlign: TextAlign.center,
            style: const TextStyle(
              color: _HomePageState._textSecondary,
              fontSize: 13,
              fontWeight: FontWeight.w600,
            ),
          ),
        ],
      ),
    );
  }
}

class _BuildSortSheet extends StatefulWidget {
  const _BuildSortSheet({
    required this.builds,
    required this.onReorder,
  });

  final List<InternalBuild> builds;
  final ReorderCallback onReorder;

  @override
  State<_BuildSortSheet> createState() => _BuildSortSheetState();
}

class _BuildSortSheetState extends State<_BuildSortSheet> {
  late List<InternalBuild> _builds;

  @override
  void initState() {
    super.initState();
    _builds = [...widget.builds];
  }

  void _handleReorder(int oldIndex, int newIndex) {
    if (oldIndex < 0 || oldIndex >= _builds.length) {
      return;
    }

    final adjustedIndex = newIndex > oldIndex ? newIndex - 1 : newIndex;
    if (adjustedIndex < 0 ||
        adjustedIndex >= _builds.length ||
        adjustedIndex == oldIndex) {
      return;
    }

    setState(() {
      final movingBuild = _builds.removeAt(oldIndex);
      _builds.insert(adjustedIndex, movingBuild);
    });
    widget.onReorder(oldIndex, newIndex);
  }

  @override
  Widget build(BuildContext context) {
    final sheetHeight = MediaQuery.sizeOf(context).height * .62;

    return Container(
      key: const ValueKey('build-sort-sheet'),
      height: sheetHeight,
      decoration: const BoxDecoration(
        color: Colors.white,
        borderRadius: BorderRadius.vertical(top: Radius.circular(24)),
      ),
      child: Column(
        children: [
          const SizedBox(height: 8),
          Container(
            width: 38,
            height: 4,
            decoration: BoxDecoration(
              color: const Color(0xFFD7DBE4),
              borderRadius: BorderRadius.circular(999),
            ),
          ),
          Padding(
            padding: const EdgeInsets.fromLTRB(18, 16, 8, 10),
            child: Row(
              children: [
                const Expanded(
                  child: Text(
                    '调整应用排序',
                    style: TextStyle(
                      color: _HomePageState._textPrimary,
                      fontSize: 19,
                      fontWeight: FontWeight.w800,
                    ),
                  ),
                ),
                IconButton(
                  tooltip: '关闭排序',
                  onPressed: () => Navigator.of(context).pop(),
                  icon: const Icon(Icons.close_rounded),
                ),
              ],
            ),
          ),
          Expanded(
            child: ReorderableListView.builder(
              key: const ValueKey('build-sort-list'),
              padding: const EdgeInsets.fromLTRB(16, 0, 16, 28),
              buildDefaultDragHandles: false,
              proxyDecorator: (child, index, animation) => child,
              onReorder: _handleReorder,
              itemCount: _builds.length,
              itemBuilder: (context, index) {
                final build = _builds[index];
                return _BuildSortSheetItem(
                  key: ValueKey('sort-build-row-${build.id}'),
                  item: build,
                  index: index,
                );
              },
            ),
          ),
        ],
      ),
    );
  }
}

class _BuildSortSheetItem extends StatelessWidget {
  const _BuildSortSheetItem({
    super.key,
    required this.item,
    required this.index,
  });

  final InternalBuild item;
  final int index;

  @override
  Widget build(BuildContext context) {
    return Container(
      key: ValueKey('sort-build-card-${item.id}'),
      margin: const EdgeInsets.only(bottom: 8),
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
      decoration: BoxDecoration(
        color: const Color(0xFFF7F8FB),
        borderRadius: BorderRadius.circular(14),
        border: Border.all(color: _HomePageState._line),
      ),
      child: Row(
        children: [
          _AppIcon(item: item, size: 42, radius: 11, iconSize: 24),
          const SizedBox(width: 12),
          Expanded(
            child: Text(
              item.name,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: const TextStyle(
                color: _HomePageState._textPrimary,
                fontSize: 16,
                fontWeight: FontWeight.w800,
              ),
            ),
          ),
          const SizedBox(width: 8),
          ReorderableDragStartListener(
            index: index,
            child: SizedBox(
              key: ValueKey('sort-drag-handle-${item.id}'),
              width: 40,
              height: 40,
              child: const Icon(
                Icons.drag_handle_rounded,
                color: Color(0xFF9AA1AD),
                size: 22,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class _BuildRow extends StatelessWidget {
  const _BuildRow({
    super.key,
    required this.item,
    required this.onAction,
    required this.onOpen,
  });

  final InternalBuild item;
  final VoidCallback onAction;
  final VoidCallback onOpen;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(14, 14, 12, 14),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.center,
        children: [
          GestureDetector(
            onTap: onOpen,
            child: _AppIcon(item: item),
          ),
          const SizedBox(width: 14),
          Expanded(
            child: InkWell(
              onTap: onOpen,
              borderRadius: BorderRadius.circular(12),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    item.name,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(
                      color: _HomePageState._textPrimary,
                      fontSize: 17,
                      fontWeight: FontWeight.w800,
                    ),
                  ),
                  const SizedBox(height: 4),
                  Text(
                    '${item.version} (${item.buildNumber})',
                    style: const TextStyle(
                      color: _HomePageState._textPrimary,
                      fontSize: 14,
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                  const SizedBox(height: 8),
                  Wrap(
                    spacing: 6,
                    runSpacing: 4,
                    children: [
                      _Badge(
                        text: item.channel.label,
                        foreground: item.channel.foreground,
                        background: item.channel.background,
                      ),
                      _Badge(
                        text: item.installInfo.platform.label,
                        foreground: const Color(0xFF53606E),
                        background: const Color(0xFFF2F3F6),
                      ),
                      _Badge(
                        text: item.statusLabel,
                        foreground: _statusForeground(item.status),
                        background: _statusBackground(item.status),
                      ),
                    ],
                  ),
                  const SizedBox(height: 8),
                  Text(
                    '更新说明：${item.note}',
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(
                        color: _HomePageState._textSecondary, fontSize: 14),
                  ),
                  const SizedBox(height: 7),
                  Row(
                    children: [
                      const Icon(Icons.schedule_rounded,
                          size: 14, color: _HomePageState._textSecondary),
                      const SizedBox(width: 3),
                      Flexible(
                        child: Text(
                          item.uploadedAt,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: const TextStyle(
                              color: _HomePageState._textSecondary,
                              fontSize: 13),
                        ),
                      ),
                    ],
                  ),
                ],
              ),
            ),
          ),
          const SizedBox(width: 10),
          SizedBox(
            width: 84,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.end,
              children: [
                if (item.status == BuildStatus.installing)
                  _DownloadProgressButton(
                    progress: item.progress,
                    isPaused: item.isPaused,
                    tooltip: item.actionLabel,
                    onPressed: onAction,
                    progressKey: ValueKey('download-progress-${item.id}'),
                  )
                else
                  OutlinedButton(
                    onPressed:
                        item.status == BuildStatus.expired ? null : onAction,
                    style: OutlinedButton.styleFrom(
                      foregroundColor: _HomePageState._blue,
                      side: const BorderSide(color: _HomePageState._blue),
                      minimumSize: const Size(66, 34),
                      padding: const EdgeInsets.symmetric(horizontal: 10),
                      shape: RoundedRectangleBorder(
                          borderRadius: BorderRadius.circular(10)),
                      textStyle: const TextStyle(
                          fontSize: 14, fontWeight: FontWeight.w800),
                    ),
                    child: Text(item.actionLabel),
                  ),
                const SizedBox(height: 8),
                Row(
                  mainAxisAlignment: MainAxisAlignment.end,
                  children: [
                    IconButton(
                      onPressed: onOpen,
                      tooltip: '查看 ${item.name} 详情',
                      icon: const Icon(Icons.chevron_right_rounded,
                          color: Color(0xFFC0C5CF), size: 24),
                      padding: EdgeInsets.zero,
                      constraints:
                          const BoxConstraints.tightFor(width: 30, height: 34),
                    ),
                  ],
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  Color _statusForeground(BuildStatus status) {
    switch (status) {
      case BuildStatus.installing:
        return const Color(0xFF2478FF);
      case BuildStatus.updateAvailable:
        return const Color(0xFFB76613);
      case BuildStatus.available:
        return const Color(0xFF20864A);
      case BuildStatus.installed:
        return const Color(0xFF53606E);
      case BuildStatus.expired:
        return const Color(0xFFDC2626);
    }
  }

  Color _statusBackground(BuildStatus status) {
    switch (status) {
      case BuildStatus.installing:
        return const Color(0xFFE8F1FF);
      case BuildStatus.updateAvailable:
        return const Color(0xFFFFF1DF);
      case BuildStatus.available:
        return const Color(0xFFE7F8EE);
      case BuildStatus.installed:
        return const Color(0xFFF2F3F6);
      case BuildStatus.expired:
        return const Color(0xFFFEE2E2);
    }
  }
}

class _DownloadProgressButton extends StatelessWidget {
  const _DownloadProgressButton({
    required this.progress,
    required this.isPaused,
    required this.tooltip,
    required this.onPressed,
    required this.progressKey,
  });

  final double? progress;
  final bool isPaused;
  final String tooltip;
  final VoidCallback onPressed;
  final Key progressKey;

  @override
  Widget build(BuildContext context) {
    final progressValue = (progress ?? 0).clamp(0.0, 1.0).toDouble();
    final foreground =
        isPaused ? const Color(0xFF6B7280) : _HomePageState._blue;
    final background =
        isPaused ? const Color(0xFFF1F3F6) : const Color(0xFFE8F1FF);

    return Semantics(
      label: '下载进度',
      value: '${(progressValue * 100).round()}%',
      button: true,
      child: SizedBox.square(
        dimension: 48,
        child: Stack(
          alignment: Alignment.center,
          children: [
            SizedBox.square(
              dimension: 48,
              child: CircularProgressIndicator(
                key: progressKey,
                value: progressValue,
                strokeWidth: 3.2,
                backgroundColor: const Color(0xFFE1E5EC),
                color: foreground,
              ),
            ),
            IconButton.filledTonal(
              onPressed: onPressed,
              icon: Icon(
                isPaused ? Icons.play_arrow_rounded : Icons.pause_rounded,
              ),
              tooltip: tooltip,
              style: IconButton.styleFrom(
                foregroundColor: foreground,
                backgroundColor: background,
                fixedSize: const Size.square(38),
                padding: EdgeInsets.zero,
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _AppIcon extends StatelessWidget {
  const _AppIcon({
    required this.item,
    this.size = 56,
    this.radius = 14,
    this.iconSize = 31,
  });

  final InternalBuild item;
  final double size;
  final double radius;
  final double iconSize;

  @override
  Widget build(BuildContext context) {
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        color: item.iconColor,
        borderRadius: BorderRadius.circular(radius),
      ),
      child: Icon(item.icon, color: Colors.white, size: iconSize),
    );
  }
}

class _Badge extends StatelessWidget {
  const _Badge({
    required this.text,
    required this.foreground,
    required this.background,
  });

  final String text;
  final Color foreground;
  final Color background;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
      decoration: BoxDecoration(
          color: background, borderRadius: BorderRadius.circular(8)),
      child: Text(
        text,
        style: TextStyle(
            color: foreground, fontSize: 12, fontWeight: FontWeight.w800),
      ),
    );
  }
}

class _BottomNav extends StatelessWidget {
  const _BottomNav({
    required this.selectedIndex,
    required this.onChanged,
  });

  final int selectedIndex;
  final ValueChanged<int> onChanged;

  @override
  Widget build(BuildContext context) {
    return BottomNavigationBar(
      currentIndex: selectedIndex,
      onTap: onChanged,
      type: BottomNavigationBarType.fixed,
      selectedItemColor: _HomePageState._blue,
      unselectedItemColor: _HomePageState._textSecondary,
      selectedLabelStyle: const TextStyle(fontWeight: FontWeight.w700),
      items: const [
        BottomNavigationBarItem(
            icon: Icon(Icons.inventory_2_rounded), label: '首页'),
        BottomNavigationBarItem(
            icon: Icon(Icons.phone_iphone_rounded), label: '设备'),
        BottomNavigationBarItem(
            icon: Icon(Icons.notifications_rounded), label: '通知'),
        BottomNavigationBarItem(icon: Icon(Icons.person_rounded), label: '我的'),
      ],
    );
  }
}
