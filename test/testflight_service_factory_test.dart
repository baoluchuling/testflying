import 'package:flutter_test/flutter_test.dart';
import 'package:testflying/services/api_client.dart';
import 'package:testflying/services/mock_testflight_service.dart';
import 'package:testflying/services/remote_testflight_service.dart';
import 'package:testflying/services/testflight_service_factory.dart';

class _FakeTransport implements ApiTransport {
  @override
  Future<ApiResponse> send(ApiRequest request) async {
    return const ApiResponse(statusCode: 200, body: {});
  }
}

void main() {
  test('service config defaults to mock mode', () {
    final config = TestFlightServiceConfig.fromValues(const {});

    expect(config.mode, TestFlightServiceMode.mock);
    expect(config.environment, ApiEnvironment.development);
    expect(config.baseUrlOverride, isNull);
    expect(config.accessToken, isEmpty);
    expect(config.deviceId, 'local');
    expect(config.platform, 'ios');
  });

  test('service config parses remote api defines', () {
    final config = TestFlightServiceConfig.fromValues(const {
      TestFlightServiceConfig.serviceKey: 'remote',
      TestFlightServiceConfig.environmentKey: 'prod',
      TestFlightServiceConfig.baseUrlKey: 'https://dist.example.test',
      TestFlightServiceConfig.accessTokenKey: 'token-123',
      TestFlightServiceConfig.deviceIdKey: 'device-456',
      TestFlightServiceConfig.platformKey: 'android',
    });

    expect(config.mode, TestFlightServiceMode.remote);
    expect(config.environment, ApiEnvironment.production);
    expect(config.baseUrlOverride, 'https://dist.example.test');
    expect(config.accessToken, 'token-123');
    expect(config.deviceId, 'device-456');
    expect(config.platform, 'android');
  });

  test('service factory builds mock service by default', () {
    final service = buildTestFlightService(
      config: TestFlightServiceConfig.fromValues(const {}),
    );

    expect(service, isA<MockTestFlightService>());
  });

  test('service factory builds remote service when explicitly requested', () {
    final service = buildTestFlightService(
      config: TestFlightServiceConfig.fromValues(const {
        TestFlightServiceConfig.serviceKey: 'remote',
      }),
      transport: _FakeTransport(),
    );

    expect(service, isA<RemoteTestFlightService>());
  });
}
