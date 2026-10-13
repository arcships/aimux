// Round trips for the message-content types against the generated wire shape:
// `bindings/node/src/types/{ContentPart,FileData,FileBytes,ToolResultOutput}.ts`.
import 'package:aimux/types.dart';
import 'package:test/test.dart';

void main() {
  group('FileBytes (a base64 string)', () {
    // SharedV4FileDataData.data is Uint8Array | string upstream
    // (shared-v4-file-data.ts:7-10): one base64 string in the JSON
    // (uint8-utils.ts:12-23), never an array of ints.
    test('binary is written as base64', () {
      expect(FileBytesBinary(data: [0, 1, 2, 250, 255]).toJson(), 'AAEC+v8=');
      expect(() => FileBytes.fromJson([104, 105]), throwsFormatException);
      expect(decodeBase64Bytes('AAEC-v8'), [0, 1, 2, 250, 255]);
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
      // ImagePart.image / FilePart.data are DataContent (provider-utils
      // types/content-part.ts:42, :75): base64 strings.
      'image': {'type': 'image', 'image': 'AQID', 'mediaType': 'image/png'},
      'file': {
        'type': 'file',
        'data': 'aGk=',
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
