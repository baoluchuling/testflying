import 'package:flutter/material.dart';
import 'package:testflying/models/internal_build.dart';
import 'package:testflying/pages/build_details.dart';

class AppDetailsPage extends StatelessWidget {
  const AppDetailsPage({
    super.key,
    required this.app,
    required this.onInstall,
  });

  final InternalBuild app;
  final Future<void> Function(InternalBuild build) onInstall;

  List<InternalBuild> get _buildHistory {
    final currentBuildNumber = int.tryParse(app.buildNumber);
    final previousBuildNumber = currentBuildNumber == null
        ? '${app.buildNumber}-1'
        : '${currentBuildNumber - 1}';
    final baselineBuildNumber = currentBuildNumber == null
        ? '${app.buildNumber}-2'
        : '${currentBuildNumber - 2}';

    return [
      app,
      InternalBuild(
        id: '${app.id}-previous',
        name: app.name,
        version: app.version,
        buildNumber: previousBuildNumber,
        channel: app.channel,
        environment: app.environment,
        owner: app.owner,
        uploadedAt: '昨天 17:42',
        note: '上一轮开发环境回归包',
        status: BuildStatus.available,
        icon: app.icon,
        iconColor: app.iconColor,
        installInfo: app.installInfo,
      ),
      InternalBuild(
        id: '${app.id}-baseline',
        name: app.name,
        version: app.version,
        buildNumber: baselineBuildNumber,
        channel: BuildChannel.dev,
        environment: app.environment,
        owner: app.owner,
        uploadedAt: '周五 18:10',
        note: '稳定基线包，用于问题复现',
        status: BuildStatus.installed,
        icon: app.icon,
        iconColor: app.iconColor,
        installInfo: app.installInfo,
      ),
    ];
  }

  @override
  Widget build(BuildContext context) {
    final buildHistory = _buildHistory;

    return Scaffold(
      backgroundColor: const Color(0xFFF5F6FA),
      appBar: AppBar(
        backgroundColor: const Color(0xFFF5F6FA),
        elevation: 0,
        title: const Text('应用详情'),
      ),
      body: SafeArea(
        child: ListView(
          padding: const EdgeInsets.fromLTRB(16, 12, 16, 24),
          children: [
            _AppHeroCard(app: app),
            const SizedBox(height: 16),
            _MetricGrid(
              items: [
                _MetricItem(
                  label: '最新版本',
                  value: app.version,
                  icon: Icons.new_releases_rounded,
                ),
                _MetricItem(
                  label: '最近构建',
                  value: app.buildNumber,
                  icon: Icons.inventory_2_rounded,
                ),
                _MetricItem(
                  label: '构建记录',
                  value: '${buildHistory.length}',
                  icon: Icons.history_rounded,
                ),
                const _MetricItem(
                  label: '可测设备',
                  value: '18',
                  icon: Icons.phone_iphone_rounded,
                ),
              ],
            ),
            const SizedBox(height: 16),
            _InfoSection(
              title: '应用信息',
              children: [
                _InfoRow(label: '应用 ID', value: app.id),
                _InfoRow(label: '默认环境', value: app.environment),
                _InfoRow(label: '安装平台', value: app.installInfo.platform.label),
                if (app.installInfo.minOsVersion != null)
                  _InfoRow(label: '最低系统', value: app.installInfo.minOsVersion!),
                if (app.installInfo.expiresAt != null)
                  _InfoRow(label: '链接有效期', value: app.installInfo.expiresAt!),
              ],
            ),
            const SizedBox(height: 16),
            const _InfoSection(
              title: '测试状态',
              children: [
                _StatusTile(
                  icon: Icons.verified_user_rounded,
                  title: '签名状态',
                  value: '企业签名有效，剩余 5 天',
                  color: Color(0xFF20864A),
                ),
                Divider(height: 18, color: Color(0xFFE7E9EF)),
                _StatusTile(
                  icon: Icons.bug_report_rounded,
                  title: '崩溃日志',
                  value: '近 24 小时 0 条新崩溃',
                  color: Color(0xFF2478FF),
                ),
              ],
            ),
            const SizedBox(height: 16),
            _InfoSection(
              title: '最近构建',
              children: [
                const SizedBox(height: 10),
                for (var index = 0; index < buildHistory.length; index++) ...[
                  _BuildVersionRow(
                    build: buildHistory[index],
                    isLatest: index == 0,
                    onOpen: () =>
                        _openBuildDetails(context, buildHistory[index]),
                  ),
                  if (index != buildHistory.length - 1)
                    const Divider(height: 18, color: Color(0xFFE7E9EF)),
                ],
              ],
            ),
            const SizedBox(height: 22),
            FilledButton.icon(
              onPressed: _canInstall(app) ? () => onInstall(app) : null,
              icon: const Icon(Icons.download_rounded),
              label: Text('安装最新构建 ${app.version}'),
              style: FilledButton.styleFrom(
                minimumSize: const Size.fromHeight(48),
                textStyle:
                    const TextStyle(fontSize: 16, fontWeight: FontWeight.w800),
              ),
            ),
          ],
        ),
      ),
    );
  }

  void _openBuildDetails(BuildContext context, InternalBuild build) {
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (context) => BuildDetailsPage(
          build: build,
          onInstall: onInstall,
        ),
      ),
    );
  }

  bool _canInstall(InternalBuild build) {
    return build.status != BuildStatus.expired &&
        build.installInfo.isInstallable;
  }
}

class _AppHeroCard extends StatelessWidget {
  const _AppHeroCard({required this.app});

  final InternalBuild app;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.all(18),
      decoration: BoxDecoration(
        color: Colors.white,
        border: Border.all(color: const Color(0xFFE7E9EF)),
        borderRadius: BorderRadius.circular(16),
      ),
      child: Row(
        children: [
          Container(
            width: 68,
            height: 68,
            decoration: BoxDecoration(
              color: app.iconColor,
              borderRadius: BorderRadius.circular(16),
            ),
            child: Icon(app.icon, color: Colors.white, size: 36),
          ),
          const SizedBox(width: 16),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  app.name,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(
                    color: Color(0xFF10131A),
                    fontSize: 24,
                    fontWeight: FontWeight.w800,
                  ),
                ),
                const SizedBox(height: 6),
                Text(
                  '${app.version} (${app.buildNumber}) · ${app.channel.label}',
                  style: const TextStyle(
                    color: Color(0xFF7B8190),
                    fontSize: 15,
                    fontWeight: FontWeight.w700,
                  ),
                ),
                const SizedBox(height: 10),
                Wrap(
                  spacing: 6,
                  runSpacing: 6,
                  children: [
                    _Badge(
                      text: app.channel.label,
                      foreground: app.channel.foreground,
                      background: app.channel.background,
                    ),
                    _Badge(
                      text: app.statusLabel,
                      foreground: const Color(0xFF2478FF),
                      background: const Color(0xFFE8F1FF),
                    ),
                    _Badge(
                      text: app.installInfo.platform.label,
                      foreground: const Color(0xFF53606E),
                      background: const Color(0xFFF2F3F6),
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
}

class _MetricGrid extends StatelessWidget {
  const _MetricGrid({required this.items});

  final List<_MetricItem> items;

  @override
  Widget build(BuildContext context) {
    return GridView.count(
      crossAxisCount: 2,
      childAspectRatio: 2.5,
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
    return Container(
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(
        color: Colors.white,
        border: Border.all(color: const Color(0xFFE7E9EF)),
        borderRadius: BorderRadius.circular(16),
      ),
      child: Row(
        children: [
          Icon(icon, color: const Color(0xFF2478FF), size: 24),
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
                    color: Color(0xFF7B8190),
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
                    color: Color(0xFF10131A),
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

class _InfoSection extends StatelessWidget {
  const _InfoSection({
    required this.title,
    required this.children,
  });

  final String title;
  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.all(16),
      decoration: BoxDecoration(
        color: Colors.white,
        border: Border.all(color: const Color(0xFFE7E9EF)),
        borderRadius: BorderRadius.circular(16),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            title,
            style: const TextStyle(
              color: Color(0xFF10131A),
              fontSize: 18,
              fontWeight: FontWeight.w800,
            ),
          ),
          ...children,
        ],
      ),
    );
  }
}

class _InfoRow extends StatelessWidget {
  const _InfoRow({
    required this.label,
    required this.value,
  });

  final String label;
  final String value;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(top: 12),
      child: Row(
        children: [
          SizedBox(
            width: 76,
            child: Text(
              label,
              style: const TextStyle(
                color: Color(0xFF7B8190),
                fontSize: 14,
                fontWeight: FontWeight.w600,
              ),
            ),
          ),
          Expanded(
            child: Text(
              value,
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
              textAlign: TextAlign.right,
              style: const TextStyle(
                color: Color(0xFF10131A),
                fontSize: 15,
                fontWeight: FontWeight.w700,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class _StatusTile extends StatelessWidget {
  const _StatusTile({
    required this.icon,
    required this.title,
    required this.value,
    required this.color,
  });

  final IconData icon;
  final String title;
  final String value;
  final Color color;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(top: 12),
      child: Row(
        children: [
          Icon(icon, color: color, size: 24),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  title,
                  style: const TextStyle(
                    color: Color(0xFF10131A),
                    fontSize: 15,
                    fontWeight: FontWeight.w800,
                  ),
                ),
                const SizedBox(height: 3),
                Text(
                  value,
                  style: const TextStyle(
                    color: Color(0xFF7B8190),
                    fontSize: 13,
                    fontWeight: FontWeight.w600,
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

class _BuildVersionRow extends StatelessWidget {
  const _BuildVersionRow({
    required InternalBuild build,
    required this.isLatest,
    required this.onOpen,
  }) : item = build;

  final InternalBuild item;
  final bool isLatest;
  final VoidCallback onOpen;

  @override
  Widget build(BuildContext context) {
    return InkWell(
      onTap: onOpen,
      borderRadius: BorderRadius.circular(12),
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: 6),
        child: Row(
          children: [
            Container(
              width: 42,
              height: 42,
              decoration: BoxDecoration(
                color: const Color(0xFFE8F1FF),
                borderRadius: BorderRadius.circular(12),
              ),
              child: Icon(
                isLatest ? Icons.new_releases_rounded : Icons.history_rounded,
                color: const Color(0xFF2478FF),
              ),
            ),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Row(
                    children: [
                      Flexible(
                        child: Text(
                          '${item.version} (${item.buildNumber})',
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: const TextStyle(
                            color: Color(0xFF10131A),
                            fontSize: 16,
                            fontWeight: FontWeight.w800,
                          ),
                        ),
                      ),
                      if (isLatest) ...[
                        const SizedBox(width: 8),
                        const _Badge(
                          text: '最新',
                          foreground: Color(0xFF2478FF),
                          background: Color(0xFFE8F1FF),
                        ),
                      ],
                    ],
                  ),
                  const SizedBox(height: 3),
                  Text(
                    '${item.channel.label} · ${item.uploadedAt} · ${item.statusLabel}',
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(
                      color: Color(0xFF7B8190),
                      fontSize: 13,
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                ],
              ),
            ),
            const SizedBox(width: 8),
            const Icon(
              Icons.chevron_right_rounded,
              color: Color(0xFF98A2B3),
            ),
          ],
        ),
      ),
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
