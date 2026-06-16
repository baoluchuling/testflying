import 'package:testflying/services/api_client.dart';
import 'package:testflying/services/install_launcher.dart';
import 'package:testflying/services/mock_testflight_service.dart';
import 'package:testflying/services/remote_testflight_service.dart';
import 'package:testflying/services/testflight_service.dart';

enum TestFlightServiceMode {
  mock,
  remote;

  static TestFlightServiceMode fromName(String value) {
    final normalized = value.trim().toLowerCase();
    return switch (normalized) {
      'remote' || 'api' || 'http' => TestFlightServiceMode.remote,
      _ => TestFlightServiceMode.mock,
    };
  }
}

class TestFlightServiceConfig {
  const TestFlightServiceConfig({
    required this.mode,
    required this.environment,
    required this.baseUrlOverride,
    required this.accessToken,
    required this.deviceId,
    required this.platform,
  });

  factory TestFlightServiceConfig.fromEnvironment() {
    return TestFlightServiceConfig.fromValues(const {
      serviceKey: String.fromEnvironment(serviceKey, defaultValue: 'mock'),
      environmentKey:
          String.fromEnvironment(environmentKey, defaultValue: 'development'),
      baseUrlKey: String.fromEnvironment(baseUrlKey),
      accessTokenKey: String.fromEnvironment(accessTokenKey),
      deviceIdKey: String.fromEnvironment(deviceIdKey, defaultValue: 'local'),
      platformKey: String.fromEnvironment(platformKey, defaultValue: 'ios'),
    });
  }

  factory TestFlightServiceConfig.fromValues(Map<String, String> values) {
    final baseUrl = values[baseUrlKey]?.trim();
    return TestFlightServiceConfig(
      mode: TestFlightServiceMode.fromName(values[serviceKey] ?? 'mock'),
      environment: ApiEnvironment.fromName(
        values[environmentKey] ?? 'development',
      ),
      baseUrlOverride: baseUrl == null || baseUrl.isEmpty ? null : baseUrl,
      accessToken: values[accessTokenKey] ?? '',
      deviceId: _nonEmpty(values[deviceIdKey], fallback: 'local'),
      platform: _nonEmpty(values[platformKey], fallback: 'ios'),
    );
  }

  static const serviceKey = 'TESTFLYING_SERVICE';
  static const environmentKey = 'TESTFLYING_API_ENV';
  static const baseUrlKey = 'TESTFLYING_API_BASE_URL';
  static const accessTokenKey = 'TESTFLYING_ACCESS_TOKEN';
  static const deviceIdKey = 'TESTFLYING_DEVICE_ID';
  static const platformKey = 'TESTFLYING_CLIENT_PLATFORM';

  final TestFlightServiceMode mode;
  final ApiEnvironment environment;
  final String? baseUrlOverride;
  final String accessToken;
  final String deviceId;
  final String platform;
}

TestFlightService buildTestFlightService({
  TestFlightServiceConfig? config,
  ApiTransport? transport,
  InstallLauncher installLauncher = const UrlInstallLauncher(),
}) {
  final resolvedConfig = config ?? TestFlightServiceConfig.fromEnvironment();
  return switch (resolvedConfig.mode) {
    TestFlightServiceMode.mock => MockTestFlightService(
        installLauncher: installLauncher,
      ),
    TestFlightServiceMode.remote => RemoteTestFlightService(
        apiClient: ApiClient(
          environment: resolvedConfig.environment,
          baseUrlOverride: resolvedConfig.baseUrlOverride,
          context: ApiClientContext(
            accessToken: resolvedConfig.accessToken,
            deviceId: resolvedConfig.deviceId,
            platform: resolvedConfig.platform,
          ),
          transport: transport ?? HttpApiTransport(),
        ),
        installLauncher: installLauncher,
      ),
  };
}

String _nonEmpty(String? value, {required String fallback}) {
  final trimmed = value?.trim();
  return trimmed == null || trimmed.isEmpty ? fallback : trimmed;
}
