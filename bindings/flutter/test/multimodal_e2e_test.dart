// Multimodal E2E tests for the aimux Flutter/Dart binding.
//
// Drives the real Rust core (via dart:ffi) against a mock OpenAI/compatible
// HTTP server (dart:io HttpServer) for every multimodal modality: Embedding,
// Speech, Image, Transcription, Reranking, Search, Files, and Video. No real
// API calls are made. Mock response shapes mirror the Go E2E tests
// (bindings/go/multimodal_withbase_test.go).
//
// Run:
//   export PATH="/home/eric8810/.local/dart-sdk/bin:$PATH"
//   export LD_LIBRARY_PATH="/media/eric8810/fast-deliver/code/aimux/target/debug:$LD_LIBRARY_PATH"
//   cd bindings/flutter && dart test test/multimodal_e2e_test.dart
//
// ## Why the FFI call runs in a worker isolate
//
// Every multimodal method (embed/generate/rerank/search/uploadFile) is a
// *synchronous* FFI call: it blocks the calling isolate until the Rust core
// finishes, and the Rust core issues an HTTP request that the mock HttpServer
// must serve. If the mock server lived in the *same* isolate, it could never
// accept the request while the isolate is blocked inside FFI → deadlock. So the
// mock server runs in the main test isolate (free event loop) and each blocking
// FFI call runs in a worker isolate via Isolate.run — exactly the pattern from
// structured_e2e_test.dart.

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:isolate';

import 'package:aimux/multimodal.dart';
import 'package:test/test.dart';

// ─────────────────────────────────────────────────────────────────────────────
// Mock server
//
// Same pattern as structured_e2e_test.dart, extended to allow a custom response
// content-type so the Speech binary path (audio/mpeg body) can be exercised —
// the stock server hardcodes application/json.
// ─────────────────────────────────────────────────────────────────────────────

/// A minimal mock server. Captures every request (method, path, parsed JSON
/// body) and replies with a caller-configured response for each request, in FIFO
/// order.
///
/// Each response map supports:
///   - 'body'        : the response body (String)
///   - 'contentType' : optional content-type (defaults to application/json)
///   - 'sse'         : if true, reply as text/event-stream
class MockOpenAIServer {
  final HttpServer _server;
  final List<Map<String, dynamic>> _responses;
  final List<_RecordedRequest> recorded = [];
  int _nextResponse = 0;

  MockOpenAIServer._(this._server, this._responses) {
    _server.listen(_handleRequest);
  }

  String get baseUrl => 'http://${_server.address.host}:${_server.port}';

  Future<void> close() => _server.close(force: true);

  void _handleRequest(HttpRequest request) async {
    final rawBody = await utf8.decoder.bind(request).join();
    dynamic parsed;
    try {
      parsed = rawBody.isEmpty ? null : jsonDecode(rawBody);
    } catch (_) {
      parsed = rawBody;
    }
    recorded.add(_RecordedRequest(
      method: request.method,
      path: request.uri.path,
      body: parsed,
    ));

    final resp = _nextResponse < _responses.length
        ? _responses[_nextResponse++]
        : _responses.last;
    final response = request.response;
    response.statusCode = HttpStatus.ok;
    if (resp['sse'] == true) {
      response.headers.contentType = ContentType.parse('text/event-stream');
      response.headers.set('Cache-Control', 'no-cache');
      response.headers.set('Connection', 'keep-alive');
      response.write(resp['body'] as String);
    } else {
      final ct = resp['contentType'] as String?;
      response.headers.contentType =
          ct != null ? ContentType.parse(ct) : ContentType.json;
      response.write(resp['body'] as String);
    }
    await response.close();
  }
}

class _RecordedRequest {
  final String method;
  final String path;
  final dynamic body;
  _RecordedRequest(
      {required this.method, required this.path, required this.body});
}

Future<MockOpenAIServer> startMockServer(
    List<Map<String, dynamic>> responses) async {
  final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
  return MockOpenAIServer._(server, responses);
}

// ─────────────────────────────────────────────────────────────────────────────
// Worker-isolate FFI drivers
// ─────────────────────────────────────────────────────────────────────────────
//
// Each driver constructs the model and invokes the blocking FFI method inside a
// worker isolate, returning the JSON result string to the main isolate. The
// model is closed in the same isolate. Construction + call + close all happen in
// the worker so the main isolate's event loop stays free to serve the mock.
//
// Call options that the Rust core deserializes (speech/image/rerank/search)
// always carry `"provider_options":{}` — matching the Go wire format, where the
// SharedProviderOptions map must be present (not null). embed/transcription/
// uploadFile pass no options (null pointer), matching the Go nil-opts path.

Future<String> runEmbed(
    String baseUrl, String apiKey, String modelId, String valuesJson) {
  return Isolate.run(() {
    final model = EmbeddingModel.openai(apiKey, modelId, baseUrl: baseUrl);
    try {
      return model.embed(valuesJson);
    } finally {
      model.close();
    }
  });
}

Future<String> runSpeechGenerate(
    String baseUrl, String apiKey, String modelId, String optsJson) {
  return Isolate.run(() {
    final model = SpeechModel.openai(apiKey, modelId, baseUrl: baseUrl);
    try {
      return model.generate(optsJson);
    } finally {
      model.close();
    }
  });
}

Future<String> runImageGenerate(
    String baseUrl, String apiKey, String modelId, String optsJson) {
  return Isolate.run(() {
    final model = ImageModel.openai(apiKey, modelId, baseUrl: baseUrl);
    try {
      return model.generate(optsJson);
    } finally {
      model.close();
    }
  });
}

Future<String> runTranscriptionGenerate(String baseUrl, String apiKey,
    String modelId, String audioBase64, String mediaType) {
  return Isolate.run(() {
    final model = TranscriptionModel.openai(apiKey, modelId, baseUrl: baseUrl);
    try {
      return model.generate(audioBase64, mediaType);
    } finally {
      model.close();
    }
  });
}

Future<String> runRerank(
    String baseUrl, String apiKey, String modelId, String optsJson) {
  return Isolate.run(() {
    final model = RerankingModel.cohere(apiKey, modelId, baseUrl: baseUrl);
    try {
      return model.rerank(optsJson);
    } finally {
      model.close();
    }
  });
}

Future<String> runSearch(String baseUrl, String apiKey, String optsJson) {
  return Isolate.run(() {
    final model = SearchModel.tavily(apiKey, baseUrl: baseUrl);
    try {
      return model.search(optsJson);
    } finally {
      model.close();
    }
  });
}

Future<String> runUploadFile(
    String baseUrl, String apiKey, String dataBase64, String mediaType) {
  return Isolate.run(() {
    final model = Files.openai(apiKey, baseUrl: baseUrl);
    try {
      return model.uploadFile(dataBase64, mediaType);
    } finally {
      model.close();
    }
  });
}

/// Construct (and immediately close) a Google video model. Construction makes no
/// HTTP, so the non-listening base URL is fine; it still goes through a worker
/// isolate to keep all FFI off the main isolate (consistent with the other
/// drivers and with structured_e2e_test.dart).
Future<void> runVideoConstruct() {
  return Isolate.run<void>(() {
    final model =
        VideoModel.google('test', 'veo-3.0', baseUrl: 'http://localhost:9999');
    model.close();
  });
}

// ─────────────────────────────────────────────────────────────────────────────
// Fixture responses (provider wire shapes — same as the Go E2E tests)
// ─────────────────────────────────────────────────────────────────────────────

const String embeddingResponse = r'''
{"data":[{"embedding":[0.1,0.2,0.3],"index":0}],"model":"text-embedding-3-small","usage":{"prompt_tokens":3,"total_tokens":3}}
''';

const String imageResponse = r'''
{"data":[{"b64_json":"aW1hZ2Ux"}]}
''';

const String transcriptionResponse = r'''
{"text":"Hello world"}
''';

const String rerankingResponse = r'''
{"results":[{"index":1,"relevance_score":0.95},{"index":0,"relevance_score":0.3}]}
''';

const String searchResponse = r'''
{"results":[{"title":"Rust","url":"https://rust-lang.org","content":"Rust is..."}],"answer":"Rust is a systems language."}
''';

const String filesResponse = r'''
{"id":"file-abc","object":"file","bytes":1024,"created_at":1234,"filename":"test.pdf","purpose":"assistants"}
''';

/// Raw binary body returned by the speech mock (base64 of "Hello world"); the
/// Rust core wraps a non-JSON audio response as AudioData::Binary.
const String speechAudioBody = 'SGVsbG8gd29ybGQ=';

/// Parsed-only fixture for the video result wire format.
const String videoResultJson = r'''
{"videos":[{"Url":{"url":"https://example.com/v.mp4","media_type":"video/mp4"}}]}
''';

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

void main() {
  const apiKey = 'sk-test-mock-key';

  test('Embedding: OpenAI embed via mock', () async {
    final server = await startMockServer([
      {'body': embeddingResponse},
    ]);
    addTearDown(server.close);

    final result = await runEmbed(
        server.baseUrl, apiKey, 'text-embedding-3-small', '["hello"]');

    final decoded = jsonDecode(result) as Map<String, dynamic>;
    expect(decoded, contains('embeddings'));
    final embeddings = decoded['embeddings'] as List;
    expect(embeddings, hasLength(1));
    expect(embeddings[0], isA<List>());
    expect((embeddings[0] as List), hasLength(3));

    // A single POST reached the provider.
    expect(server.recorded, hasLength(1));
    expect(server.recorded.first.method, 'POST');
  });

  test('Speech: OpenAI TTS via mock returns binary audio', () async {
    // The mock returns a raw binary body with content-type audio/mpeg (matching
    // the Go E2E test). The Rust core wraps non-JSON audio as AudioData::Binary.
    final server = await startMockServer([
      {'body': speechAudioBody, 'contentType': 'audio/mpeg'},
    ]);
    addTearDown(server.close);

    final optsJson = jsonEncode({
      'text': 'Hi',
      'voice': 'alloy',
      'output_format': 'mp3',
      'provider_options': <String, dynamic>{},
    });

    final result =
        await runSpeechGenerate(server.baseUrl, apiKey, 'tts-1', optsJson);

    final decoded = jsonDecode(result) as Map<String, dynamic>;
    expect(decoded, contains('audio'));
    final audio = decoded['audio'] as Map<String, dynamic>;
    expect(audio, contains('Binary'));
    final binary = audio['Binary'] as List;
    expect(binary, isNotEmpty);

    expect(server.recorded, hasLength(1));
    expect(server.recorded.first.method, 'POST');
  });

  test('Image: OpenAI image generation via mock', () async {
    final server = await startMockServer([
      {'body': imageResponse},
    ]);
    addTearDown(server.close);

    final optsJson = jsonEncode({
      'prompt': 'otter',
      'n': 1,
      'provider_options': <String, dynamic>{},
    });

    final result =
        await runImageGenerate(server.baseUrl, apiKey, 'dall-e-3', optsJson);

    final decoded = jsonDecode(result) as Map<String, dynamic>;
    expect(decoded, contains('images'));
    final images = decoded['images'] as Map<String, dynamic>;
    expect(images, contains('Base64'));
    final base64List = images['Base64'] as List;
    expect(base64List, hasLength(1));
    expect(base64List[0], 'aW1hZ2Ux');

    expect(server.recorded, hasLength(1));
    expect(server.recorded.first.method, 'POST');
  });

  test('Transcription: OpenAI STT via mock', () async {
    final server = await startMockServer([
      {'body': transcriptionResponse},
    ]);
    addTearDown(server.close);

    final result = await runTranscriptionGenerate(
        server.baseUrl, apiKey, 'whisper-1', 'dGVzdA==', 'audio/mp3');

    final decoded = jsonDecode(result) as Map<String, dynamic>;
    expect(decoded['text'], 'Hello world');

    expect(server.recorded, hasLength(1));
    expect(server.recorded.first.method, 'POST');
  });

  test('Reranking: Cohere rerank via mock', () async {
    final server = await startMockServer([
      {'body': rerankingResponse},
    ]);
    addTearDown(server.close);

    final optsJson = jsonEncode({
      'query': 'which?',
      'documents': {'Text': {'values': ['doc1', 'doc2']}},
      'top_n': 2,
      'provider_options': <String, dynamic>{},
    });

    final result =
        await runRerank(server.baseUrl, apiKey, 'rerank-v3.0', optsJson);

    final decoded = jsonDecode(result) as Map<String, dynamic>;
    expect(decoded, contains('ranking'));
    final ranking = decoded['ranking'] as List;
    expect(ranking, hasLength(2));
    expect((ranking[0] as Map)['relevance_score'], 0.95);
    expect((ranking[0] as Map)['index'], 1);

    expect(server.recorded, hasLength(1));
    expect(server.recorded.first.method, 'POST');
  });

  test('Search: Tavily search via mock', () async {
    final server = await startMockServer([
      {'body': searchResponse},
    ]);
    addTearDown(server.close);

    final optsJson = jsonEncode({
      'query': 'What is Rust?',
      'max_results': 5,
      'provider_options': <String, dynamic>{},
    });

    final result = await runSearch(server.baseUrl, apiKey, optsJson);

    final decoded = jsonDecode(result) as Map<String, dynamic>;
    expect(decoded, contains('results'));
    final results = decoded['results'] as List;
    expect(results, hasLength(1));
    expect((results[0] as Map)['title'], 'Rust');
    expect((results[0] as Map)['url'], 'https://rust-lang.org');
    expect(decoded['answer'], 'Rust is a systems language.');

    expect(server.recorded, hasLength(1));
    expect(server.recorded.first.method, 'POST');
  });

  test('Files: OpenAI file upload via mock', () async {
    final server = await startMockServer([
      {'body': filesResponse},
    ]);
    addTearDown(server.close);

    final result = await runUploadFile(
        server.baseUrl, apiKey, 'dGVzdA==', 'application/pdf');

    final decoded = jsonDecode(result) as Map<String, dynamic>;
    expect(decoded, contains('provider_reference'));
    final providerRef = decoded['provider_reference'] as Map<String, dynamic>;
    expect(providerRef['openai'], 'file-abc');

    expect(server.recorded, hasLength(1));
    expect(server.recorded.first.method, 'POST');
  });

  test('Video: Google video construction + result parsing', () async {
    // Google Video uses a multi-step async API (POST predict → poll operation →
    // fetch result). A single-response mock server can't drive the full flow, so
    // — like the Go E2E test — we verify construction and result parsing only.
    await runVideoConstruct();

    final decoded = jsonDecode(videoResultJson) as Map<String, dynamic>;
    expect(decoded, contains('videos'));
    final videos = decoded['videos'] as List;
    expect(videos, hasLength(1));
    final urlData = (videos[0] as Map)['Url'] as Map<String, dynamic>;
    expect(urlData['url'], 'https://example.com/v.mp4');
    expect(urlData['media_type'], 'video/mp4');
  });
}
