import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:testflying/models/internal_build.dart';
import 'package:testflying/models/workspace_data.dart';

const _blue = Color(0xFF2478FF);
const _textPrimary = Color(0xFF10131A);
const _textSecondary = Color(0xFF7B8190);
const _line = Color(0xFFE7E9EF);

class CollapsibleTabScrollView extends StatefulWidget {
  const CollapsibleTabScrollView({
    super.key,
    required this.title,
    required this.titleKeyPrefix,
    required this.children,
    this.trailing,
  });

  final String title;
  final String titleKeyPrefix;
  final List<Widget> children;
  final Widget? trailing;

  @override
  State<CollapsibleTabScrollView> createState() =>
      _CollapsibleTabScrollViewState();
}

class _CollapsibleTabScrollViewState extends State<CollapsibleTabScrollView> {
  final ScrollController _scrollController = ScrollController();
  double _collapseProgress = 0;

  @override
  void initState() {
    super.initState();
    _scrollController.addListener(_syncTitleState);
  }

  @override
  void dispose() {
    _scrollController.removeListener(_syncTitleState);
    _scrollController.dispose();
    super.dispose();
  }

  void _syncTitleState() {
    final nextProgress = _scrollController.hasClients
        ? (_scrollController.offset / 52).clamp(0.0, 1.0)
        : 0.0;
    if ((nextProgress - _collapseProgress).abs() < .01 &&
        nextProgress != 0 &&
        nextProgress != 1) {
      return;
    }
    setState(() => _collapseProgress = nextProgress);
  }

  @override
  Widget build(BuildContext context) {
    final prefix = widget.titleKeyPrefix;
    final largeTitleOpacity = 1 - _collapseProgress;
    final smallTitleProgress = _interval(
      _collapseProgress,
      begin: .5,
      end: 1,
      curve: Curves.easeOutCubic,
    );

    return CustomScrollView(
      key: ValueKey('$prefix-scroll-view'),
      controller: _scrollController,
      slivers: [
        SliverAppBar(
          key: ValueKey('$prefix-title-bar'),
          pinned: true,
          centerTitle: true,
          automaticallyImplyLeading: false,
          backgroundColor: const Color(0xFFF5F6FA),
          surfaceTintColor: Colors.transparent,
          elevation: 0,
          expandedHeight: 92,
          toolbarHeight: 48,
          titleSpacing: 16,
          title: smallTitleProgress == 0
              ? SizedBox(key: ValueKey('$prefix-small-title-empty'))
              : Opacity(
                  key: ValueKey('$prefix-small-title-opacity'),
                  opacity: smallTitleProgress,
                  child: Transform(
                    key: ValueKey('$prefix-small-title-flip'),
                    alignment: Alignment.bottomCenter,
                    transform: Matrix4.identity()
                      ..setEntry(3, 2, .001)
                      ..rotateX((1 - smallTitleProgress) * math.pi / 2.15),
                    child: _SmallCollapsibleTitle(
                      key: ValueKey('$prefix-small-title'),
                      title: widget.title,
                    ),
                  ),
                ),
          actions: [
            if (widget.trailing != null)
              Padding(
                padding: const EdgeInsets.only(right: 8),
                child: widget.trailing!,
              ),
          ],
          flexibleSpace: Stack(
            key: ValueKey('$prefix-collapsing-title'),
            children: [
              Positioned(
                left: 16,
                right: 16,
                bottom: 10,
                child: largeTitleOpacity == 0
                    ? SizedBox(key: ValueKey('$prefix-large-title-empty'))
                    : Opacity(
                        key: ValueKey('$prefix-large-title-opacity'),
                        opacity: largeTitleOpacity,
                        child: _LargeCollapsibleTitle(
                          key: ValueKey('$prefix-large-title'),
                          title: widget.title,
                        ),
                      ),
              ),
            ],
          ),
        ),
        SliverPadding(
          padding: const EdgeInsets.fromLTRB(16, 14, 16, 18),
          sliver: SliverToBoxAdapter(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: widget.children,
            ),
          ),
        ),
      ],
    );
  }

  static double _interval(
    double value, {
    required double begin,
    required double end,
    Curve curve = Curves.linear,
  }) {
    final progress = ((value - begin) / (end - begin)).clamp(0.0, 1.0);
    return curve.transform(progress);
  }
}

class _SmallCollapsibleTitle extends StatelessWidget {
  const _SmallCollapsibleTitle({
    super.key,
    required this.title,
  });

  final String title;

  @override
  Widget build(BuildContext context) {
    return Text(
      title,
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
      style: const TextStyle(
        color: _textPrimary,
        fontSize: 20,
        fontWeight: FontWeight.w800,
        letterSpacing: 0,
      ),
    );
  }
}

class _LargeCollapsibleTitle extends StatelessWidget {
  const _LargeCollapsibleTitle({
    super.key,
    required this.title,
  });

  final String title;

  @override
  Widget build(BuildContext context) {
    return Text(
      title,
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
      style: const TextStyle(
        color: _textPrimary,
        fontSize: 34,
        height: 1.05,
        fontWeight: FontWeight.w800,
        letterSpacing: 0,
      ),
    );
  }
}

class AppCatalogPage extends StatelessWidget {
  const AppCatalogPage({
    super.key,
    required this.builds,
    required this.onOpen,
    required this.onAction,
    required this.onMessage,
  });

  final List<InternalBuild> builds;
  final ValueChanged<InternalBuild> onOpen;
  final ValueChanged<InternalBuild> onAction;
  final ValueChanged<String> onMessage;

  @override
  Widget build(BuildContext context) {
    final installed =
        builds.where((build) => build.status == BuildStatus.installed).length;
    final updates = builds
        .where((build) => build.status == BuildStatus.updateAvailable)
        .length;

    return _TabScrollView(
      children: [
        _PageHeader(
          title: '应用',
          trailing: IconButton(
            tooltip: '扫描新版本',
            onPressed: () => onMessage('已扫描最新构建'),
            icon: const Icon(Icons.sync_rounded, color: _blue, size: 30),
          ),
        ),
        const SizedBox(height: 16),
        _MetricGrid(
          items: [
            _MetricItem(
                label: '全部应用',
                value: '${builds.length}',
                icon: Icons.apps_rounded),
            _MetricItem(
                label: '待更新',
                value: '$updates',
                icon: Icons.system_update_alt_rounded),
            _MetricItem(
                label: '已安装',
                value: '$installed',
                icon: Icons.check_circle_rounded),
            const _MetricItem(
                label: '环境分类', value: '2', icon: Icons.account_tree_rounded),
          ],
        ),
        const SizedBox(height: 16),
        const _SearchField(),
        const SizedBox(height: 18),
        _SectionHeader(
            title: '安装队列',
            action: '查看任务',
            onAction: () => onMessage('安装任务已同步')),
        const SizedBox(height: 10),
        _QueuePanel(
          builds: builds
              .where((build) => build.status == BuildStatus.installing)
              .toList(),
        ),
        const SizedBox(height: 20),
        _SectionHeader(
            title: '应用清单',
            action: '批量更新',
            onAction: () => onMessage('已加入批量更新队列')),
        const SizedBox(height: 10),
        _Panel(
          padding: EdgeInsets.zero,
          child: Column(
            children: [
              for (var index = 0; index < builds.length; index++) ...[
                _CatalogRow(
                  item: builds[index],
                  onOpen: () => onOpen(builds[index]),
                  onAction: () => onAction(builds[index]),
                ),
                if (index != builds.length - 1)
                  const Divider(height: 1, indent: 78, color: _line),
              ],
            ],
          ),
        ),
      ],
    );
  }
}

class DevicesPage extends StatelessWidget {
  const DevicesPage({
    super.key,
    required this.currentDevice,
    required this.devices,
    required this.developerAccount,
    required this.onMessage,
  });

  final TestDevice currentDevice;
  final List<TestDevice> devices;
  final DeveloperAccount? developerAccount;
  final ValueChanged<String> onMessage;

  @override
  Widget build(BuildContext context) {
    return CollapsibleTabScrollView(
      title: '设备',
      titleKeyPrefix: 'devices',
      trailing: IconButton(
        tooltip: '添加设备',
        onPressed: () => onMessage('已复制设备登记链接'),
        icon: const Icon(Icons.add_circle_rounded, color: _blue, size: 30),
      ),
      children: [
        _Panel(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              _StatusLine(
                icon: Icons.phone_iphone_rounded,
                title: '当前设备',
                value: currentDevice.name,
                badge: currentDevice.status,
                badgeColor: currentDevice.statusColor,
              ),
              const SizedBox(height: 16),
              const Divider(height: 1, color: _line),
              const SizedBox(height: 16),
              _InfoPair(label: 'UDID', value: currentDevice.udid),
              _InfoPair(label: '系统版本', value: currentDevice.osVersion),
              _InfoPair(label: '证书状态', value: currentDevice.certificateStatus),
              _InfoPair(label: '最近安装', value: currentDevice.lastInstalledAt),
            ],
          ),
        ),
        const SizedBox(height: 18),
        _SectionHeader(
            title: '设备池', action: '刷新', onAction: () => onMessage('设备池已刷新')),
        const SizedBox(height: 10),
        _Panel(
          padding: EdgeInsets.zero,
          child: Column(
            children: [
              for (var index = 0; index < devices.length; index++) ...[
                _DeviceRow(
                  name: devices[index].name,
                  owner: devices[index].owner,
                  status: devices[index].status,
                  statusColor: devices[index].statusColor,
                  detail: devices[index].detail,
                ),
                if (index != devices.length - 1)
                  const Divider(height: 1, indent: 70, color: _line),
              ],
            ],
          ),
        ),
        const SizedBox(height: 18),
        const _SectionHeader(title: '证书与权限'),
        const SizedBox(height: 10),
        _Panel(
          child: Column(
            children: [
              _ActionTile(
                icon: Icons.verified_user_rounded,
                title: '企业签名',
                subtitle:
                    developerAccount?.certificateSubtitle ?? '暂无开发者账号续费提醒',
                action: developerAccount == null ? '检查' : '更新',
                onPressed: () => onMessage(
                  developerAccount == null ? '暂无需要续费的开发者账号' : '证书更新任务已创建',
                ),
              ),
              const Divider(height: 18, color: _line),
              _ActionTile(
                icon: Icons.vpn_key_rounded,
                title: '调试权限',
                subtitle: '开发环境、线上环境可安装',
                action: '管理',
                onPressed: () => onMessage('权限管理暂存完成'),
              ),
            ],
          ),
        ),
      ],
    );
  }
}

class NotificationsPage extends StatefulWidget {
  const NotificationsPage({
    super.key,
    required this.notifications,
    required this.onMessage,
  });

  final List<AppNotification> notifications;
  final ValueChanged<String> onMessage;

  @override
  State<NotificationsPage> createState() => _NotificationsPageState();
}

class _NotificationsPageState extends State<NotificationsPage> {
  final ScrollController _scrollController = ScrollController();
  NoticeType _selectedType = NoticeType.all;
  double _collapseProgress = 0;

  List<AppNotification> get _visibleNotices {
    if (_selectedType == NoticeType.all) {
      return widget.notifications;
    }
    return widget.notifications
        .where((notice) => notice.type == _selectedType)
        .toList();
  }

  int _countFor(NoticeType type) {
    if (type == NoticeType.all) {
      return widget.notifications.length;
    }
    return widget.notifications.where((notice) => notice.type == type).length;
  }

  @override
  void initState() {
    super.initState();
    _scrollController.addListener(_syncTitleState);
  }

  @override
  void dispose() {
    _scrollController.removeListener(_syncTitleState);
    _scrollController.dispose();
    super.dispose();
  }

  void _syncTitleState() {
    final nextProgress = _scrollController.hasClients
        ? (_scrollController.offset / 58).clamp(0.0, 1.0)
        : 0.0;
    if ((nextProgress - _collapseProgress).abs() < .01 &&
        nextProgress != 0 &&
        nextProgress != 1) {
      return;
    }
    setState(() => _collapseProgress = nextProgress);
  }

  @override
  Widget build(BuildContext context) {
    final visibleNotices = _visibleNotices;
    final largeTitleOpacity = 1 - _collapseProgress;
    final smallTitleProgress = _CollapsibleTabScrollViewState._interval(
      _collapseProgress,
      begin: .5,
      end: 1,
      curve: Curves.easeOutCubic,
    );

    return CustomScrollView(
      key: const ValueKey('notification-scroll-view'),
      controller: _scrollController,
      slivers: [
        SliverAppBar(
          key: const ValueKey('notification-title-bar'),
          pinned: true,
          centerTitle: true,
          automaticallyImplyLeading: false,
          backgroundColor: const Color(0xFFF5F6FA),
          surfaceTintColor: Colors.transparent,
          elevation: 0,
          expandedHeight: 142,
          toolbarHeight: 48,
          titleSpacing: 16,
          title: smallTitleProgress == 0
              ? const SizedBox(key: ValueKey('notification-small-title-empty'))
              : Opacity(
                  key: const ValueKey('notification-small-title-opacity'),
                  opacity: smallTitleProgress,
                  child: Transform(
                    key: const ValueKey('notification-small-title-flip'),
                    alignment: Alignment.bottomCenter,
                    transform: Matrix4.identity()
                      ..setEntry(3, 2, .001)
                      ..rotateX((1 - smallTitleProgress) * math.pi / 2.15),
                    child: const Text(
                      '通知',
                      key: ValueKey('notification-small-title'),
                      style: TextStyle(
                        color: _textPrimary,
                        fontSize: 20,
                        fontWeight: FontWeight.w800,
                        letterSpacing: 0,
                      ),
                    ),
                  ),
                ),
          actions: [
            Padding(
              padding: const EdgeInsets.only(right: 8),
              child: TextButton(
                onPressed: () => widget.onMessage('通知已全部标记为已读'),
                child: const Text('全部已读'),
              ),
            ),
          ],
          flexibleSpace: Stack(
            key: const ValueKey('notification-collapsing-title'),
            children: [
              Positioned(
                left: 16,
                right: 118,
                bottom: 76,
                child: largeTitleOpacity == 0
                    ? const SizedBox(
                        key: ValueKey('notification-large-title-empty'),
                      )
                    : Opacity(
                        key: const ValueKey('notification-large-title-opacity'),
                        opacity: largeTitleOpacity,
                        child: const Text(
                          '通知',
                          key: ValueKey('notification-large-title'),
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(
                            color: _textPrimary,
                            fontSize: 34,
                            height: 1.05,
                            fontWeight: FontWeight.w800,
                            letterSpacing: 0,
                          ),
                        ),
                      ),
              ),
            ],
          ),
          bottom: PreferredSize(
            preferredSize: const Size.fromHeight(64),
            child: Container(
              color: const Color(0xFFF5F6FA),
              padding: const EdgeInsets.fromLTRB(16, 6, 16, 6),
              child: _NotificationTypeFilter(
                selectedType: _selectedType,
                countFor: _countFor,
                onSelect: (type) => setState(() => _selectedType = type),
              ),
            ),
          ),
        ),
        SliverPadding(
          padding: const EdgeInsets.fromLTRB(16, 18, 16, 18),
          sliver: SliverList(
            delegate: SliverChildListDelegate(
              [
                for (final section in ['今天', '昨天'])
                  ..._buildNoticeSection(
                    section,
                    visibleNotices
                        .where((notice) => notice.section == section)
                        .toList(),
                  ),
              ],
            ),
          ),
        ),
      ],
    );
  }

  List<Widget> _buildNoticeSection(
    String section,
    List<AppNotification> notices,
  ) {
    if (notices.isEmpty) {
      return const [];
    }

    return [
      _SectionHeader(title: section),
      const SizedBox(height: 10),
      _Panel(
        padding: EdgeInsets.zero,
        child: Column(
          children: [
            for (var index = 0; index < notices.length; index++) ...[
              _NoticeRow(
                icon: notices[index].icon,
                title: notices[index].title,
                subtitle: notices[index].subtitle,
                tag: notices[index].tag,
                tagColor: notices[index].tagColor,
              ),
              if (index != notices.length - 1)
                const Divider(height: 1, indent: 70, color: _line),
            ],
          ],
        ),
      ),
      const SizedBox(height: 18),
    ];
  }
}

class ProfilePage extends StatelessWidget {
  const ProfilePage({
    super.key,
    required this.profile,
    required this.onMessage,
  });

  final UserProfile profile;
  final ValueChanged<String> onMessage;

  @override
  Widget build(BuildContext context) {
    return CollapsibleTabScrollView(
      title: '我的',
      titleKeyPrefix: 'profile',
      trailing: IconButton(
        tooltip: '账号设置',
        onPressed: () => onMessage('账号设置已打开'),
        icon: const Icon(Icons.settings_rounded, color: _blue, size: 30),
      ),
      children: [
        _Panel(
          child: Row(
            children: [
              CircleAvatar(
                radius: 30,
                backgroundColor: const Color(0xFFE8F1FF),
                child: Text(
                  profile.initial,
                  style: const TextStyle(
                    color: _blue,
                    fontSize: 26,
                    fontWeight: FontWeight.w800,
                  ),
                ),
              ),
              const SizedBox(width: 14),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      profile.name,
                      style: const TextStyle(
                        color: _textPrimary,
                        fontSize: 22,
                        fontWeight: FontWeight.w800,
                      ),
                    ),
                    const SizedBox(height: 4),
                    Text(
                      profile.subtitle,
                      style: const TextStyle(
                        color: _textSecondary,
                        fontSize: 14,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                  ],
                ),
              ),
            ],
          ),
        ),
        const SizedBox(height: 18),
        _MetricGrid(
          items: [
            for (final metric in profile.metrics)
              _MetricItem(
                label: metric.label,
                value: metric.value,
                icon: metric.icon,
              ),
          ],
        ),
        const SizedBox(height: 18),
        const _SectionHeader(title: '工作台'),
        const SizedBox(height: 10),
        _Panel(
          padding: EdgeInsets.zero,
          child: Column(
            children: [
              for (var index = 0; index < profile.actions.length; index++) ...[
                _SettingsRow(
                  icon: profile.actions[index].icon,
                  title: profile.actions[index].title,
                  subtitle: profile.actions[index].subtitle,
                  onPressed: () => onMessage(profile.actions[index].message),
                ),
                if (index != profile.actions.length - 1)
                  const Divider(height: 1, indent: 70, color: _line),
              ],
            ],
          ),
        ),
        const SizedBox(height: 18),
        const _SectionHeader(title: '偏好'),
        const SizedBox(height: 10),
        _Panel(
          child: Column(
            children: [
              for (var index = 0;
                  index < profile.preferences.length;
                  index++) ...[
                _SwitchRow(
                  title: profile.preferences[index].title,
                  subtitle: profile.preferences[index].subtitle,
                  value: profile.preferences[index].value,
                  onChanged: (_) =>
                      onMessage(profile.preferences[index].message),
                ),
                if (index != profile.preferences.length - 1)
                  const Divider(height: 18, color: _line),
              ],
            ],
          ),
        ),
      ],
    );
  }
}

class _TabScrollView extends StatelessWidget {
  const _TabScrollView({required this.children});

  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    return ListView(
      padding: const EdgeInsets.fromLTRB(16, 16, 16, 18),
      children: children,
    );
  }
}

class _PageHeader extends StatelessWidget {
  const _PageHeader({
    required this.title,
    this.trailing,
  });

  final String title;
  final Widget? trailing;

  @override
  Widget build(BuildContext context) {
    return Row(
      children: [
        Expanded(
          child: Text(
            title,
            style: const TextStyle(
              color: _textPrimary,
              fontSize: 34,
              fontWeight: FontWeight.w800,
              letterSpacing: 0,
            ),
          ),
        ),
        if (trailing != null) trailing!,
      ],
    );
  }
}

class _Panel extends StatelessWidget {
  const _Panel({
    required this.child,
    this.padding = const EdgeInsets.all(16),
  });

  final Widget child;
  final EdgeInsetsGeometry padding;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: padding,
      decoration: BoxDecoration(
        color: Colors.white,
        border: Border.all(color: _line),
        borderRadius: BorderRadius.circular(16),
      ),
      child: child,
    );
  }
}

class _MetricGrid extends StatelessWidget {
  const _MetricGrid({required this.items});

  final List<_MetricItem> items;

  @override
  Widget build(BuildContext context) {
    return GridView.count(
      crossAxisCount: 2,
      childAspectRatio: 2.6,
      shrinkWrap: true,
      physics: const NeverScrollableScrollPhysics(),
      crossAxisSpacing: 10,
      mainAxisSpacing: 10,
      children: items,
    );
  }
}

class _MetricItem extends StatelessWidget {
  const _MetricItem({
    required this.label,
    required this.value,
    required this.icon,
  });

  final String label;
  final String value;
  final IconData icon;

  @override
  Widget build(BuildContext context) {
    return _Panel(
      padding: const EdgeInsets.all(12),
      child: Row(
        children: [
          Icon(icon, color: _blue, size: 24),
          const SizedBox(width: 10),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                Text(
                  label,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(
                    color: _textSecondary,
                    fontSize: 12,
                    fontWeight: FontWeight.w700,
                  ),
                ),
                const SizedBox(height: 4),
                Text(
                  value,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(
                    color: _textPrimary,
                    fontSize: 20,
                    fontWeight: FontWeight.w800,
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _SearchField extends StatelessWidget {
  const _SearchField();

  @override
  Widget build(BuildContext context) {
    return TextField(
      decoration: InputDecoration(
        hintText: '搜索应用或版本',
        prefixIcon: const Icon(Icons.search_rounded),
        filled: true,
        fillColor: Colors.white,
        contentPadding:
            const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
        border: OutlineInputBorder(
          borderRadius: BorderRadius.circular(14),
          borderSide: const BorderSide(color: _line),
        ),
        enabledBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(14),
          borderSide: const BorderSide(color: _line),
        ),
      ),
    );
  }
}

class _SectionHeader extends StatelessWidget {
  const _SectionHeader({
    required this.title,
    this.action,
    this.onAction,
  });

  final String title;
  final String? action;
  final VoidCallback? onAction;

  @override
  Widget build(BuildContext context) {
    return Row(
      children: [
        Text(
          title,
          style: const TextStyle(
            color: _textPrimary,
            fontSize: 24,
            fontWeight: FontWeight.w800,
          ),
        ),
        const Spacer(),
        if (action != null)
          TextButton(
            onPressed: onAction,
            child: Text(action!),
          ),
      ],
    );
  }
}

class _QueuePanel extends StatelessWidget {
  const _QueuePanel({required this.builds});

  final List<InternalBuild> builds;

  @override
  Widget build(BuildContext context) {
    if (builds.isEmpty) {
      return const _Panel(
        child: Text(
          '当前没有安装任务',
          style: TextStyle(color: _textSecondary, fontWeight: FontWeight.w600),
        ),
      );
    }

    final build = builds.first;
    return _Panel(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Expanded(
                child: Text(
                  build.name,
                  style: const TextStyle(
                    color: _textPrimary,
                    fontSize: 18,
                    fontWeight: FontWeight.w800,
                  ),
                ),
              ),
              Text(
                '${((build.progress ?? 0) * 100).round()}%',
                style: const TextStyle(
                  color: _textSecondary,
                  fontWeight: FontWeight.w800,
                ),
              ),
            ],
          ),
          const SizedBox(height: 10),
          LinearProgressIndicator(
            value: build.progress,
            minHeight: 7,
            color: build.isPaused ? const Color(0xFF98A2B3) : _blue,
            backgroundColor: const Color(0xFFE7EAF0),
          ),
          const SizedBox(height: 10),
          Text(
            build.environment,
            style: const TextStyle(
              color: _textSecondary,
              fontWeight: FontWeight.w600,
            ),
          ),
        ],
      ),
    );
  }
}

class _CatalogRow extends StatelessWidget {
  const _CatalogRow({
    required this.item,
    required this.onOpen,
    required this.onAction,
  });

  final InternalBuild item;
  final VoidCallback onOpen;
  final VoidCallback onAction;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(14, 14, 12, 14),
      child: Row(
        children: [
          _AppIcon(item: item),
          const SizedBox(width: 14),
          Expanded(
            child: InkWell(
              onTap: onOpen,
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    item.name,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(
                      color: _textPrimary,
                      fontSize: 17,
                      fontWeight: FontWeight.w800,
                    ),
                  ),
                  const SizedBox(height: 4),
                  Text(
                    '${item.version} (${item.buildNumber})',
                    style: const TextStyle(
                      color: _textPrimary,
                      fontSize: 14,
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                  const SizedBox(height: 8),
                  Wrap(
                    spacing: 6,
                    children: [
                      _Badge(
                        text: item.channel.label,
                        foreground: item.channel.foreground,
                        background: item.channel.background,
                      ),
                    ],
                  ),
                ],
              ),
            ),
          ),
          const SizedBox(width: 10),
          OutlinedButton(
            onPressed: item.status == BuildStatus.expired ? null : onAction,
            style: OutlinedButton.styleFrom(
              foregroundColor: _blue,
              side: const BorderSide(color: _blue),
              minimumSize: const Size(64, 34),
              padding: const EdgeInsets.symmetric(horizontal: 10),
              shape: RoundedRectangleBorder(
                  borderRadius: BorderRadius.circular(10)),
              textStyle:
                  const TextStyle(fontSize: 14, fontWeight: FontWeight.w800),
            ),
            child: Text(item.actionLabel),
          ),
        ],
      ),
    );
  }
}

class _AppIcon extends StatelessWidget {
  const _AppIcon({required this.item});

  final InternalBuild item;

  @override
  Widget build(BuildContext context) {
    return Container(
      width: 50,
      height: 50,
      decoration: BoxDecoration(
        color: item.iconColor,
        borderRadius: BorderRadius.circular(13),
      ),
      child: Icon(item.icon, color: Colors.white, size: 28),
    );
  }
}

class _StatusLine extends StatelessWidget {
  const _StatusLine({
    required this.icon,
    required this.title,
    required this.value,
    required this.badge,
    required this.badgeColor,
  });

  final IconData icon;
  final String title;
  final String value;
  final String badge;
  final Color badgeColor;

  @override
  Widget build(BuildContext context) {
    return Row(
      children: [
        Icon(icon, color: _blue, size: 34),
        const SizedBox(width: 12),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                title,
                style: const TextStyle(
                  color: _textSecondary,
                  fontSize: 13,
                  fontWeight: FontWeight.w700,
                ),
              ),
              const SizedBox(height: 4),
              Text(
                value,
                style: const TextStyle(
                  color: _textPrimary,
                  fontSize: 22,
                  fontWeight: FontWeight.w800,
                ),
              ),
            ],
          ),
        ),
        _Badge(
            text: badge,
            foreground: badgeColor,
            background: badgeColor.withValues(alpha: .12)),
      ],
    );
  }
}

class _InfoPair extends StatelessWidget {
  const _InfoPair({
    required this.label,
    required this.value,
  });

  final String label;
  final String value;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(top: 10),
      child: Row(
        children: [
          SizedBox(
            width: 76,
            child: Text(
              label,
              style: const TextStyle(
                color: _textSecondary,
                fontWeight: FontWeight.w700,
              ),
            ),
          ),
          Expanded(
            child: Text(
              value,
              textAlign: TextAlign.right,
              overflow: TextOverflow.ellipsis,
              style: const TextStyle(
                color: _textPrimary,
                fontWeight: FontWeight.w700,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class _DeviceRow extends StatelessWidget {
  const _DeviceRow({
    required this.name,
    required this.owner,
    required this.status,
    required this.statusColor,
    required this.detail,
  });

  final String name;
  final String owner;
  final String status;
  final Color statusColor;
  final String detail;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.all(14),
      child: Row(
        children: [
          Container(
            width: 46,
            height: 46,
            decoration: BoxDecoration(
              color: const Color(0xFFE8F1FF),
              borderRadius: BorderRadius.circular(12),
            ),
            child: const Icon(Icons.phone_iphone_rounded, color: _blue),
          ),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  name,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(
                    color: _textPrimary,
                    fontSize: 16,
                    fontWeight: FontWeight.w800,
                  ),
                ),
                const SizedBox(height: 4),
                Text(
                  '$owner · $detail',
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(
                    color: _textSecondary,
                    fontSize: 13,
                    fontWeight: FontWeight.w600,
                  ),
                ),
              ],
            ),
          ),
          _Badge(
            text: status,
            foreground: statusColor,
            background: statusColor.withValues(alpha: .12),
          ),
        ],
      ),
    );
  }
}

class _ActionTile extends StatelessWidget {
  const _ActionTile({
    required this.icon,
    required this.title,
    required this.subtitle,
    required this.action,
    required this.onPressed,
  });

  final IconData icon;
  final String title;
  final String subtitle;
  final String action;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    return Row(
      children: [
        Icon(icon, color: _blue, size: 28),
        const SizedBox(width: 12),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                title,
                style: const TextStyle(
                  color: _textPrimary,
                  fontSize: 16,
                  fontWeight: FontWeight.w800,
                ),
              ),
              const SizedBox(height: 4),
              Text(
                subtitle,
                style: const TextStyle(
                  color: _textSecondary,
                  fontSize: 13,
                  fontWeight: FontWeight.w600,
                ),
              ),
            ],
          ),
        ),
        TextButton(onPressed: onPressed, child: Text(action)),
      ],
    );
  }
}

class _NotificationTypeFilter extends StatelessWidget {
  const _NotificationTypeFilter({
    required this.selectedType,
    required this.countFor,
    required this.onSelect,
  });

  final NoticeType selectedType;
  final int Function(NoticeType type) countFor;
  final ValueChanged<NoticeType> onSelect;

  @override
  Widget build(BuildContext context) {
    return Container(
      key: const ValueKey('notification-type-filter'),
      padding: const EdgeInsets.all(4),
      decoration: BoxDecoration(
        color: Colors.white,
        border: Border.all(color: _line),
        borderRadius: BorderRadius.circular(16),
      ),
      child: Row(
        children: [
          for (final type in NoticeType.values)
            Expanded(
              child: _NotificationTypeItem(
                type: type,
                count: countFor(type),
                selected: selectedType == type,
                onTap: () => onSelect(type),
              ),
            ),
        ],
      ),
    );
  }
}

class _NotificationTypeItem extends StatelessWidget {
  const _NotificationTypeItem({
    required this.type,
    required this.count,
    required this.selected,
    required this.onTap,
  });

  final NoticeType type;
  final int count;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final foreground = selected ? Colors.white : _textSecondary;
    final valueColor = selected ? Colors.white : _textPrimary;

    return Material(
      color: selected ? _blue : Colors.transparent,
      borderRadius: BorderRadius.circular(12),
      clipBehavior: Clip.antiAlias,
      child: InkWell(
        key: ValueKey('notification-filter-${type.name}'),
        onTap: onTap,
        borderRadius: BorderRadius.circular(12),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 9),
          child: Row(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              Icon(type.icon, color: foreground, size: 17),
              const SizedBox(width: 4),
              Text(
                type.label,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  color: foreground,
                  fontSize: 12,
                  fontWeight: FontWeight.w800,
                ),
              ),
              const SizedBox(width: 4),
              Text(
                '$count',
                style: TextStyle(
                  color: valueColor,
                  fontSize: 13,
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

class _NoticeRow extends StatelessWidget {
  const _NoticeRow({
    required this.icon,
    required this.title,
    required this.subtitle,
    required this.tag,
    required this.tagColor,
  });

  final IconData icon;
  final String title;
  final String subtitle;
  final String tag;
  final Color tagColor;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.all(14),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Container(
            width: 44,
            height: 44,
            decoration: BoxDecoration(
              color: tagColor.withValues(alpha: .12),
              borderRadius: BorderRadius.circular(12),
            ),
            child: Icon(icon, color: tagColor),
          ),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  title,
                  style: const TextStyle(
                    color: _textPrimary,
                    fontSize: 16,
                    fontWeight: FontWeight.w800,
                  ),
                ),
                const SizedBox(height: 5),
                Text(
                  subtitle,
                  style: const TextStyle(
                    color: _textSecondary,
                    fontSize: 13,
                    fontWeight: FontWeight.w600,
                  ),
                ),
                const SizedBox(height: 8),
                _Badge(
                  text: tag,
                  foreground: tagColor,
                  background: tagColor.withValues(alpha: .12),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _SettingsRow extends StatelessWidget {
  const _SettingsRow({
    required this.icon,
    required this.title,
    required this.subtitle,
    required this.onPressed,
  });

  final IconData icon;
  final String title;
  final String subtitle;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    return ListTile(
      leading: Icon(icon, color: _blue),
      title: Text(
        title,
        style: const TextStyle(
          color: _textPrimary,
          fontWeight: FontWeight.w800,
        ),
      ),
      subtitle: Text(subtitle),
      trailing: const Icon(Icons.chevron_right_rounded),
      onTap: onPressed,
    );
  }
}

class _SwitchRow extends StatelessWidget {
  const _SwitchRow({
    required this.title,
    required this.subtitle,
    required this.value,
    required this.onChanged,
  });

  final String title;
  final String subtitle;
  final bool value;
  final ValueChanged<bool> onChanged;

  @override
  Widget build(BuildContext context) {
    return Row(
      children: [
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                title,
                style: const TextStyle(
                  color: _textPrimary,
                  fontSize: 16,
                  fontWeight: FontWeight.w800,
                ),
              ),
              const SizedBox(height: 4),
              Text(
                subtitle,
                style: const TextStyle(
                  color: _textSecondary,
                  fontSize: 13,
                  fontWeight: FontWeight.w600,
                ),
              ),
            ],
          ),
        ),
        Switch(value: value, onChanged: onChanged),
      ],
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
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
      decoration: BoxDecoration(
        color: background,
        borderRadius: BorderRadius.circular(8),
      ),
      child: Text(
        text,
        style: TextStyle(
          color: foreground,
          fontSize: 12,
          fontWeight: FontWeight.w800,
        ),
      ),
    );
  }
}
