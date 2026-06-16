import 'package:flutter/material.dart';
import 'package:testflying/models/internal_build.dart';

class BuildDetailsPage extends StatelessWidget {
  const BuildDetailsPage({
    super.key,
    required InternalBuild build,
    required this.onInstall,
  }) : detail = build;

  final InternalBuild detail;
  final Future<void> Function(InternalBuild build) onInstall;

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: const Color(0xFFF5F6FA),
      appBar: AppBar(
        backgroundColor: const Color(0xFFF5F6FA),
        elevation: 0,
        title: const Text('构建详情'),
      ),
      body: SafeArea(
        child: ListView(
          padding: const EdgeInsets.fromLTRB(16, 12, 16, 24),
          children: [
            _HeroCard(build: detail),
            const SizedBox(height: 16),
            _InfoSection(
              title: '构建信息',
              children: [
                _InfoRow(label: '负责人', value: detail.owner),
                _InfoRow(label: '上传时间', value: detail.uploadedAt),
                _InfoRow(label: '运行环境', value: detail.environment),
                _InfoRow(label: '构建编号', value: detail.buildNumber),
                _InfoRow(
                    label: '安装平台', value: detail.installInfo.platform.label),
                if (detail.installInfo.minOsVersion != null)
                  _InfoRow(
                    label: '最低系统',
                    value: detail.installInfo.minOsVersion!,
                  ),
                if (detail.installInfo.manifestUrl != null)
                  _InfoRow(
                    label: 'Manifest',
                    value: detail.installInfo.manifestUrl!,
                  ),
                if (detail.installInfo.downloadUrl != null)
                  _InfoRow(
                    label: detail.installInfo.platform == InstallPlatform.ios
                        ? 'IPA'
                        : 'APK',
                    value: detail.installInfo.downloadUrl!,
                  ),
              ],
            ),
            const SizedBox(height: 16),
            _InfoSection(
              title: '更新说明',
              children: [
                Padding(
                  padding: const EdgeInsets.only(top: 10),
                  child: Text(
                    detail.note,
                    style: const TextStyle(
                      color: Color(0xFF10131A),
                      fontSize: 16,
                      height: 1.35,
                    ),
                  ),
                ),
              ],
            ),
            const SizedBox(height: 22),
            FilledButton.icon(
              onPressed: _canInstall(detail) ? () => onInstall(detail) : null,
              icon: const Icon(Icons.download_rounded),
              label: Text(detail.actionLabel),
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

  bool _canInstall(InternalBuild build) {
    return build.status != BuildStatus.expired &&
        build.installInfo.isInstallable;
  }
}

class _HeroCard extends StatelessWidget {
  const _HeroCard({required InternalBuild build}) : detail = build;

  final InternalBuild detail;

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
            width: 64,
            height: 64,
            decoration: BoxDecoration(
              color: detail.iconColor,
              borderRadius: BorderRadius.circular(14),
            ),
            child: Icon(detail.icon, color: Colors.white, size: 34),
          ),
          const SizedBox(width: 16),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  detail.name,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(
                    color: Color(0xFF10131A),
                    fontSize: 22,
                    fontWeight: FontWeight.w800,
                  ),
                ),
                const SizedBox(height: 6),
                Text(
                  '${detail.version} (${detail.buildNumber})',
                  style: const TextStyle(
                    color: Color(0xFF10131A),
                    fontSize: 15,
                    fontWeight: FontWeight.w600,
                  ),
                ),
                const SizedBox(height: 10),
                Wrap(
                  spacing: 6,
                  runSpacing: 6,
                  children: [
                    _Badge(
                      text: detail.channel.label,
                      foreground: detail.channel.foreground,
                      background: detail.channel.background,
                    ),
                    _Badge(
                      text: detail.installInfo.platform.label,
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
