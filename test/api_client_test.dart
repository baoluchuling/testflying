import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:testflying/services/api_client.dart';

class _FakeTransport implements ApiTransport {
  _FakeTransport(this.responses);

  final List<ApiResponse> responses;
  final requests = <ApiRequest>[];

  @override
  Future<ApiResponse> send(ApiRequest request) async {
    requests.add(request);
    return responses.removeAt(0);
  }
}

void main() {
  test('api client builds environment url and context headers', () async {
    final transport = _FakeTransport([
      const ApiResponse(statusCode: 200, body: {'ok': true}),
    ]);
    final client = ApiClient(
      environment: ApiEnvironment.staging,
      context: const ApiClientContext(
        accessToken: 'token-123',
        deviceId: 'device-456',
        platform: 'ios',
      ),
      transport: transport,
    );

    final response = await client.post(
      '/v1/test-distribution/workspace',
      body: {'ping': true},
    );

    expect(response['ok'], isTrue);
    expect(transport.requests, hasLength(1));
    final request = transport.requests.single;
    expect(request.method, 'POST');
    expect(
      request.url.toString(),
      'https://staging-test-distribution.internal.example.com/v1/test-distribution/workspace',
    );
    expect(request.headers['Authorization'], 'Bearer token-123');
    expect(request.headers['X-Device-ID'], 'device-456');
    expect(request.headers['X-Client-Platform'], 'ios');
    expect(request.headers['Content-Type'], 'application/json');
    expect(request.body, {'ping': true});
  });

  test('api client can override environment base url', () async {
    final transport = _FakeTransport([
      const ApiResponse(statusCode: 200, body: {'ok': true}),
    ]);
    final client = ApiClient(
      environment: ApiEnvironment.production,
      baseUrlOverride: 'https://dist.example.test/api',
      context: const ApiClientContext(
        accessToken: 'token-123',
        deviceId: 'device-456',
        platform: 'ios',
      ),
      transport: transport,
    );

    await client.get('/workspace');

    expect(
      transport.requests.single.url.toString(),
      'https://dist.example.test/api/workspace',
    );
  });

  test('http transport sends json body and decodes json response', () async {
    final transport = HttpApiTransport(
      client: MockClient((request) async {
        expect(request.method, 'PUT');
        expect(
          request.url.toString(),
          'https://dist.example.test/v1/test-distribution/users/me/build-sort-order',
        );
        expect(request.headers['Content-Type'], 'application/json');

        final requestBody = jsonDecode(request.body);
        expect(requestBody, {
          'buildIds': ['aurora'],
        });

        return http.Response(
          '{"ok":true}',
          200,
          headers: {'content-type': 'application/json'},
        );
      }),
    );
    final client = ApiClient(
      environment: ApiEnvironment.development,
      baseUrlOverride: 'https://dist.example.test',
      context: const ApiClientContext(
        accessToken: 'token-123',
        deviceId: 'device-456',
        platform: 'ios',
      ),
      transport: transport,
    );

    final response = await client.put(
      '/v1/test-distribution/users/me/build-sort-order',
      body: {
        'buildIds': ['aurora'],
      },
    );

    expect(response, {'ok': true});
  });

  test('api client maps error response into ApiException', () async {
    final transport = _FakeTransport([
      const ApiResponse(
        statusCode: 403,
        body: {
          'code': 'device_not_registered',
          'message': '当前设备未登记',
          'retryable': false,
        },
      ),
    ]);
    final client = ApiClient(
      environment: ApiEnvironment.development,
      context: const ApiClientContext(
        accessToken: 'token-123',
        deviceId: 'device-456',
        platform: 'ios',
      ),
      transport: transport,
    );

    try {
      await client.get('/v1/test-distribution/workspace');
      fail('Expected ApiException');
    } on ApiException catch (error) {
      expect(error.code, 'device_not_registered');
      expect(error.message, '当前设备未登记');
      expect(error.statusCode, 403);
      expect(error.retryable, isFalse);
    }
  });
}
