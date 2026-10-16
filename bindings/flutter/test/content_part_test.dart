// Round trips for the message-content types against the generated wire shape:
// `bindings/node/src/types/{ContentPart,FileData,FileBytes,ToolResultOutput}.ts`.
import 'dart:convert';
import 'package:aimux/types.dart';
import 'package:test/test.dart';

void main() {
  group('FileBytes (a base64 string)', () {
    // SharedV4FileDataData.data is Uint8Array | string upstream
    // (shared-v4-file-data.ts:7-10): one base64 string in the JSON
    // (uint8-utils.ts:12-23), never an array of ints.
    test('binary is written as base64', () {
      expect(FileBytes(base64Encode([0, 1, 2, 250, 255])).toJson(), 'AAEC+v8=');
      expect(() => FileBytes.fromJson([104, 105]), throwsFormatException);
      expect(base64Decode(base64.normalize('AAEC-v8')), [0, 1, 2, 250, 255]);
    });

    test('base64 is a bare string', () {
      expect(FileBytes('aGk=').toJson(), 'aGk=');
      expect(FileBytes.fromJson('aGk=').value, 'aGk=');
    });
  });

  group('FileData ("type"-tagged)', () {
    test('data / url / reference / text round-trip', () {
      final cases = <Map<String, dynamic>>[
        {'type': 'data', 'data': 'aGk='},
        {'type': 'url', 'url': 'https://example.com/f.png', 'originalUrl': 'x'},
        {
          'type': 'reference',
          'reference': {'openai': 'file-abc'}
        },
        {'type': 'text', 'text': 'hello world'},
      ];
      for (final json in cases) {
        expect(FileData.fromJson(json).toJson(), json);
      }
      expect(FileData.fromJson(cases[0]), isA<FileDataData>());
      expect(FileData.fromJson(cases[1]), isA<FileDataUrl>());
      expect(FileData.fromJson(cases[2]), isA<FileDataReference>());
      expect(FileData.fromJson(cases[3]), isA<FileDataText>());
    });
  });

  group('ContentPart round-trip', () {
    // One wire document per variant; optional fields are absent, not null.
    final cases = <String, Map<String, dynamic>>{
      'text': {'type': 'text', 'text': 'hello'},
      // ImagePart.image / FilePart.data are DataContent (provider-utils
      // types/content-part.ts:42, :75): base64 strings.
      'image': {'type': 'image', 'image': 'AQID', 'mediaType': 'image/png'},
      // FilePart.data is FileData | DataContent | URL | ProviderReference
      // (content-part.ts:75, file-data.ts:44-48).
      'file': {
        'type': 'file',
        'data': 'aGk=',
        'mediaType': 'application/pdf',
        'filename': 'doc.pdf',
      },
      'file tagged data': {
        'type': 'file',
        'data': {'type': 'data', 'data': 'aGk='},
        'mediaType': 'image/png',
      },
      'file tagged url': {
        'type': 'file',
        'data': {'type': 'url', 'url': 'https://example.com/f.png'},
        'mediaType': 'image/png',
      },
      'file tagged reference': {
        'type': 'file',
        'data': {
          'type': 'reference',
          'reference': {'openai': 'file-abc'},
        },
        'mediaType': 'application/pdf',
      },
      'file bare reference': {
        'type': 'file',
        'data': {'openai': 'file-abc'},
        'mediaType': 'application/pdf',
      },
      'reasoning-file bare': {
        'type': 'reasoning-file',
        'data': 'aGk=',
        'mediaType': 'image/png',
      },
      'reasoning': {
        'type': 'reasoning',
        'text': 'thinking...',
        'signature': 'sig'
      },
      'reasoning-file': {
        'type': 'reasoning-file',
        'data': {'type': 'data', 'data': 'aGk='},
        'mediaType': 'image/png',
      },
      'custom': {
        'type': 'custom',
        'kind': 'k',
        'providerOptions': {
          'a': {'b': 1}
        },
      },
      'tool-approval-request': {
        'type': 'tool-approval-request',
        'approvalId': 'ap1',
        'toolCallId': 'call_1',
        'isAutomatic': true,
      },
      'tool-call': {
        'type': 'tool-call',
        'toolCallId': 'call_1',
        'toolName': 'get_weather',
        'input': {'location': 'Tokyo'},
        'providerExecuted': true,
      },
      'tool-result': {
        'type': 'tool-result',
        'toolCallId': 'call_1',
        'toolName': 'get_weather',
        'output': {
          'type': 'json',
          'value': {'temp': 22},
        },
      },
    };

    for (final entry in cases.entries) {
      test(entry.key, () {
        final json = entry.value;
        final part = switch (json['type']) {
          'text' ||
          'image' ||
          'file' =>
            UserContentPart.fromJson(json).toJson(),
          'tool-result' => ToolContentPart.fromJson(json).toJson(),
          _ => AssistantContentPart.fromJson(json).toJson(),
        };
        expect(part, entry.value);
      });
    }

    test('file-base64 / file-url / file-reference are not AI SDK parts', () {
      for (final tag in ['file-base64', 'file-url', 'file-reference']) {
        expect(
            () => UserContentPart.fromJson({
                  'type': tag,
                  'data': 'aGk=',
                  'url': 'https://example.com/f.png',
                  'mediaType': 'image/png',
                }),
            throwsFormatException);
      }
      final bare = UserContentPart.fromJson({
        'type': 'file',
        'data': {'openai': 'file-abc'},
        'mediaType': 'application/pdf',
      }) as UserContentPartFile;
      expect(bare.data.toJson(), {'openai': 'file-abc'});
      expect(
          UserContentPart.fromJson({
            'type': 'file',
            'data': {
              'type': 'data',
              'data': base64Encode([104, 105])
            },
            'mediaType': 'text/plain',
          }).toJson(),
          {
            'type': 'file',
            'data': {'type': 'data', 'data': 'aGk='},
            'mediaType': 'text/plain',
          });
    });

    test('tool-result carries a typed output, not result/isError', () {
      final part = ToolContentPartToolResult(
        toolCallId: 'call_1',
        toolName: 'get_weather',
        output:
            ToolResultOutput.fromJson({'type': 'error-text', 'value': 'boom'}),
      );
      final json = part.toJson();
      expect(json, isNot(contains('result')));
      expect(json, isNot(contains('isError')));
      expect(json['output'], {'type': 'error-text', 'value': 'boom'});
    });

    test('ToolResultOutput variants', () {
      for (final json in <Map<String, dynamic>>[
        {'type': 'text', 'value': 'ok'},
        {
          'type': 'json',
          'value': {'a': 1}
        },
        {'type': 'execution-denied', 'reason': 'no'},
        {'type': 'execution-denied'},
        {'type': 'error-text', 'value': 'e'},
        {
          'type': 'error-json',
          'value': [1]
        },
        {
          'type': 'content',
          'value': [
            {'type': 'text', 'text': 't'}
          ]
        },
      ]) {
        expect(ToolResultOutput.fromJson(json).toJson(), json);
      }
    });

    test('unknown type is rejected', () {
      expect(() => UserContentPart.fromJson({'type': 'future-variant'}),
          throwsFormatException);
    });
  });

  group('null is kept apart from a missing value', () {
    // `body?: unknown` (language-model-v4-generate-result.ts:41, :56) and
    // `inputSchemaInput?: unknown` (tool-approval-request.ts:41): the AI SDK's
    // JSON omits undefined and keeps null.
    test('response and request body', () {
      for (final json in <Map<String, dynamic>>[
        {},
        {'body': null},
        {
          'body': {'a': null}
        },
      ]) {
        expect(ResponseInfo.fromJson(json).toJson(), json);
        expect(RequestInfo.fromJson(json).toJson(), json);
      }
      expect(RequestInfo(body: const Field.of(null)).toJson(), {'body': null});
      expect(RequestInfo().toJson(), <String, dynamic>{});
    });

    test('tool approval request inputSchemaInput', () {
      final json = {
        'type': 'tool-approval-request',
        'approvalId': 'a1',
        'toolCallId': 'c1',
        'inputSchemaInput': null,
      };
      final part = AssistantContentPart.fromJson(json)
          as AssistantContentPartToolApprovalRequest;
      expect(part.inputSchemaInput.isPresent, isTrue);
      expect(part.toJson(), json);
    });
  });
}
