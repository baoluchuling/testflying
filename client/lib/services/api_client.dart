import 'dart:convert';

import 'package:http/http.dart' as http;

enum ApiEnvironment {
  development(
    label: '开发',
    baseUrl: 'https://dev-test-distribution.internal.example.com/',
  ),
  staging(
    label: '预发',
    baseUrl: 'https://staging-test-distribution.internal.example.com/',
  ),
  production(
    label: '生产',
    baseUrl: 'https://test-distribution.internal.example.com/',
  );

  const ApiEnvironment({
    required this.label,
    required this.baseUrl,
  });

  final String label;
  final String baseUrl;

  static ApiEnvironment fromName(String value) {
    final normalized = value.trim().toLowerCase().replaceAll(
          RegExp(r'[-_]'),
          '',
        );
    return switch (normalized) {
      'dev' || 'development' => ApiEnvironment.development,
      'stage' || 'staging' || 'pre' || 'preview' => ApiEnvironment.staging,
      'prod' || 'production' => ApiEnvironment.production,
      _ => ApiEnvironment.development,
    };
  }
}

class ApiClientContext {
  const ApiClientContext({
    required this.accessToken,
    required this.deviceId,
    required this.platform,
  });

  final String accessToken;
  final String deviceId;
  final String platform;

  Map<String, String> toHeaders() {
    return {
      'Authorization': 'Bearer $accessToken',
      'X-Device-ID': deviceId,
      'X-Client-Platform': platform,
    };
  }
}

class ApiRequest {
  const ApiRequest({
    required this.method,
    required this.url,
    required this.path,
    required this.headers,
    this.body,
  });

  final String method;
  final Uri url;
  final String path;
  final Map<String, String> headers;
  final Object? body;
}

class ApiResponse {
  const ApiResponse({
    required this.statusCode,
    this.body,
  });

  final int statusCode;
  final Object? body;
}

abstract class ApiTransport {
  Future<ApiResponse> send(ApiRequest request);
}

class HttpApiTransport implements ApiTransport {
  HttpApiTransport({http.Client? client}) : _client = client ?? http.Client();

  final http.Client _client;

  @override
  Future<ApiResponse> send(ApiRequest request) async {
    final body = request.body == null ? null : jsonEncode(request.body);
    final responseFuture = switch (request.method.toUpperCase()) {
      'GET' => _client.get(request.url, headers: request.headers),
      'POST' => _client.post(
          request.url,
          headers: request.headers,
          body: body,
        ),
      'PATCH' => _client.patch(
          request.url,
          headers: request.headers,
          body: body,
        ),
      'PUT' => _client.put(
          request.url,
          headers: request.headers,
          body: body,
        ),
      final method => throw UnsupportedError('Unsupported method: $method'),
    };
    final response = await responseFuture;

    return ApiResponse(
      statusCode: response.statusCode,
      body: _decodeBody(response.body),
    );
  }
}

class ApiException implements Exception {
  const ApiException({
    required this.code,
    required this.message,
    required this.statusCode,
    required this.retryable,
  });

  factory ApiException.fromResponse(ApiResponse response) {
    final body = response.body;
    if (body is Map) {
      return ApiException(
        code: _string(body['code'], fallback: 'http_${response.statusCode}'),
        message: _string(body['message'], fallback: '请求失败'),
        statusCode: response.statusCode,
        retryable: body['retryable'] == true,
      );
    }

    return ApiException(
      code: 'http_${response.statusCode}',
      message: '请求失败',
      statusCode: response.statusCode,
      retryable: response.statusCode >= 500,
    );
  }

  final String code;
  final String message;
  final int statusCode;
  final bool retryable;

  @override
  String toString() => '$code: $message';
}

class ApiClient {
  const ApiClient({
    required this.environment,
    required this.context,
    required this.transport,
    this.baseUrlOverride,
  });

  final ApiEnvironment environment;
  final ApiClientContext context;
  final ApiTransport transport;
  final String? baseUrlOverride;

  Future<Map<String, Object?>> get(String path) {
    return _send('GET', path);
  }

  Future<Map<String, Object?>> post(
    String path, {
    Map<String, Object?>? body,
  }) {
    return _send('POST', path, body: body);
  }

  Future<Map<String, Object?>> patch(
    String path, {
    Map<String, Object?>? body,
  }) {
    return _send('PATCH', path, body: body);
  }

  Future<Map<String, Object?>> put(
    String path, {
    Map<String, Object?>? body,
  }) {
    return _send('PUT', path, body: body);
  }

  Future<Map<String, Object?>> _send(
    String method,
    String path, {
    Map<String, Object?>? body,
  }) async {
    final response = await transport.send(
      ApiRequest(
        method: method,
        url: _urlFor(path),
        path: path,
        headers: _headers(hasBody: body != null),
        body: body,
      ),
    );

    if (response.statusCode < 200 || response.statusCode >= 300) {
      throw ApiException.fromResponse(response);
    }

    final responseBody = response.body;
    if (responseBody == null) {
      return const {};
    }
    if (responseBody is Map<String, Object?>) {
      return responseBody;
    }
    if (responseBody is Map) {
      return {
        for (final entry in responseBody.entries)
          if (entry.key is String) entry.key as String: entry.value,
      };
    }

    throw const ApiException(
      code: 'invalid_response',
      message: '接口响应格式不正确',
      statusCode: 200,
      retryable: false,
    );
  }

  Uri _urlFor(String path) {
    final configuredBaseUrl = baseUrlOverride;
    final sourceBaseUrl =
        configuredBaseUrl == null || configuredBaseUrl.trim().isEmpty
            ? environment.baseUrl
            : configuredBaseUrl.trim();
    final baseUrl =
        sourceBaseUrl.endsWith('/') ? sourceBaseUrl : '$sourceBaseUrl/';
    final normalizedPath = path.replaceFirst(RegExp(r'^/'), '');
    return Uri.parse(baseUrl).resolve(normalizedPath);
  }

  Map<String, String> _headers({required bool hasBody}) {
    return {
      ...context.toHeaders(),
      'Accept': 'application/json',
      if (hasBody) 'Content-Type': 'application/json',
    };
  }
}

String _string(Object? value, {required String fallback}) {
  return value is String && value.isNotEmpty ? value : fallback;
}

Object? _decodeBody(String rawBody) {
  final trimmedBody = rawBody.trim();
  if (trimmedBody.isEmpty) {
    return null;
  }

  try {
    return jsonDecode(trimmedBody);
  } on FormatException {
    return trimmedBody;
  }
}
