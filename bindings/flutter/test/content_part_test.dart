// Round trips for the message-content types against the generated wire shape:
// `bindings/node/src/types/{ContentPart,FileData,FileBytes,ToolResultOutput}.ts`.
import 'package:aimux/types.dart';
import 'package:test/test.dart';

void main() {
  group('FileBytes (untagged: number[] | string)', () {
    test('binary is a bare array', () {
      expect(FileBytesBinary(data: [104, 105]).toJson(), [104, 105]);
      expect(FileBytes.fromJson([104, 105]), isA<FileBytesBinary>());
    });

    test('base64 is a bare string', () {
      expect(FileBytesBase64(data: 'aGk=').toJson(), 'aGk=');
      expect((FileBytes.fromJson('aGk=') as FileBytesBase64).data, 'aGk=');
    });
  });

  group('FileData ("type"-tagged)', () {
    test('data / url / reference / text round-trip', () {
      final cases = <Map<String, dynamic>>[
        {'type': 'data', 'data': 'aGk='},
        {'type': 'url', 'url': 'https://example.com/f.png', 'originalUrl': 'x'},
        {'type': 'reference', 'reference': {'openai': 'file-abc'}},
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
      'image': {'type': 'image', 'image': [1, 2, 3], 'mediaType': 'image/png'},
      'file': {
        'type': 'file',
        'data': [104, 105],
        'mediaType': 'application/pdf',
        'filename': 'doc.pdf',
      },
      'file-base64': {
        'type': 'file-base64',
        'data': 'aGk=',
        'mediaType': 'image/png',
      },
      'file-url': {
        'type': 'file-url',
        'url': 'https://example.com/f.png',
        'mediaType': 'image/png',
      },
      'file-reference': {
        'type': 'file-reference',
        'mediaType': 'application/pdf',
        'reference': {'openai': 'file-abc'},
      },
      'reasoning': {'type': 'reasoning', 'text': 'thinking...', 'signature': 'sig'},
      'reasoning-file': {
        'type': 'reasoning-file',
        'data': {'type': 'data', 'data': 'aGk='},
        'mediaType': 'image/png',
      },
      'custom': {
        'type': 'custom',
        'kind': 'k',
        'providerOptions': {'a': {'b': 1}},
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
        final part = ContentPart.fromJson(entry.value);
        expect(part, isNot(isA<ContentPartUnknown>()));
        expect(part.toJson(), entry.value);
      });
    }

    test('tool-result carries a typed output, not result/isError', () {
      final part = ContentPartToolResult(
        toolCallId: 'call_1',
        toolName: 'get_weather',
        output: ToolResultOutput.errorText('boom'),
      );
      final json = part.toJson();
      expect(json, isNot(contains('result')));
      expect(json, isNot(contains('isError')));
      expect(json['output'], {'type': 'error-text', 'value': 'boom'});
    });

    test('ToolResultOutput variants', () {
      for (final json in <Map<String, dynamic>>[
        {'type': 'text', 'value': 'ok'},
        {'type': 'json', 'value': {'a': 1}},
        {'type': 'execution-denied', 'reason': 'no'},
        {'type': 'execution-denied'},
        {'type': 'error-text', 'value': 'e'},
        {'type': 'error-json', 'value': [1]},
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

    test('unknown type passes through verbatim', () {
      final json = {'type': 'future-variant', 'data': 'something'};
      final part = ContentPart.fromJson(json);
      expect(part, isA<ContentPartUnknown>());
      expect(part.toJson(), json);
    });
  });
}
