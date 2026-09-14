import 'package:test/test.dart';
import 'package:aimux/types.dart';

void main() {
  group('FileBytes round-trip', () {
    test('Binary variant', () {
      final original = FileBytesBinary(data: [104, 105]);
      final json = original.toJson();
      expect(json, {'Binary': [104, 105]});
      final decoded = FileBytes.fromJson(json);
      expect(decoded, isA<FileBytesBinary>());
      expect((decoded as FileBytesBinary).data, [104, 105]);
    });

    test('Base64 variant', () {
      final original = FileBytesBase64(data: 'aGk=');
      final json = original.toJson();
      expect(json, {'Base64': 'aGk='});
      final decoded = FileBytes.fromJson(json);
      expect(decoded, isA<FileBytesBase64>());
      expect((decoded as FileBytesBase64).data, 'aGk=');
    });
  });

  group('FileData round-trip', () {
    test('Data variant', () {
      final original = FileDataData(data: FileBytesBase64(data: 'aGk='));
      final json = original.toJson();
      expect(json, {
        'Data': {'data': {'Base64': 'aGk='}}
      });
      final decoded = FileData.fromJson(json);
      expect(decoded, isA<FileDataData>());
      final d = decoded as FileDataData;
      expect(d.data, isA<FileBytesBase64>());
      expect((d.data as FileBytesBase64).data, 'aGk=');
    });

    test('Url variant', () {
      final original = FileDataUrl(url: 'https://example.com/f.png');
      final json = original.toJson();
      expect(json, {
        'Url': {'url': 'https://example.com/f.png'}
      });
      final decoded = FileData.fromJson(json);
      expect(decoded, isA<FileDataUrl>());
      expect((decoded as FileDataUrl).url, 'https://example.com/f.png');
    });

    test('Reference variant', () {
      final original = FileDataReference(reference: {'openai': 'file-abc'});
      final json = original.toJson();
      expect(json, {
        'Reference': {
          'reference': {'openai': 'file-abc'}
        }
      });
      final decoded = FileData.fromJson(json);
      expect(decoded, isA<FileDataReference>());
      expect((decoded as FileDataReference).reference, {'openai': 'file-abc'});
    });

    test('Text variant', () {
      final original = FileDataText(text: 'hello world');
      final json = original.toJson();
      expect(json, {
        'Text': {'text': 'hello world'}
      });
      final decoded = FileData.fromJson(json);
      expect(decoded, isA<FileDataText>());
      expect((decoded as FileDataText).text, 'hello world');
    });
  });

  group('ContentPart round-trip', () {
    test('Text variant', () {
      final original = ContentPartText(text: 'hello');
      final json = original.toJson();
      expect(json['type'], 'text');
      expect(json['text'], 'hello');
      final decoded = ContentPart.fromJson(json);
      expect(decoded, isA<ContentPartText>());
      expect((decoded as ContentPartText).text, 'hello');
    });

    test('Image variant', () {
      final original = ContentPartImage(image: [1, 2, 3], mediaType: 'image/png');
      final json = original.toJson();
      expect(json['type'], 'image');
      expect(json['image'], [1, 2, 3]);
      expect(json['media_type'], 'image/png');
      final decoded = ContentPart.fromJson(json);
      expect(decoded, isA<ContentPartImage>());
      final d = decoded as ContentPartImage;
      expect(d.image, [1, 2, 3]);
      expect(d.mediaType, 'image/png');
    });

    test('File variant — has filename', () {
      final original = ContentPartFile(
        data: [104, 105],
        mediaType: 'application/pdf',
        filename: 'doc.pdf',
      );
      final json = original.toJson();
      expect(json['type'], 'file');
      expect(json, contains('filename'));
      expect(json['filename'], 'doc.pdf');
      final decoded = ContentPart.fromJson(json);
      expect(decoded, isA<ContentPartFile>());
      final d = decoded as ContentPartFile;
      expect(d.data, [104, 105]);
      expect(d.mediaType, 'application/pdf');
      expect(d.filename, 'doc.pdf');
    });

    test('FileBase64 variant', () {
      final original = ContentPartFileBase64(
        data: 'aGk=',
        mediaType: 'image/png',
        filename: 'img.png',
      );
      final json = original.toJson();
      expect(json['type'], 'file_base64');
      expect(json['data'], 'aGk=');
      final decoded = ContentPart.fromJson(json);
      expect(decoded, isA<ContentPartFileBase64>());
      expect((decoded as ContentPartFileBase64).data, 'aGk=');
    });

    test('FileUrl variant', () {
      final original = ContentPartFileUrl(
        url: 'https://example.com/f.png',
        mediaType: 'image/png',
      );
      final json = original.toJson();
      expect(json['type'], 'file_url');
      expect(json['url'], 'https://example.com/f.png');
      final decoded = ContentPart.fromJson(json);
      expect(decoded, isA<ContentPartFileUrl>());
      expect((decoded as ContentPartFileUrl).url, 'https://example.com/f.png');
    });

    test('FileReference variant', () {
      final original = ContentPartFileReference(
        mediaType: 'application/pdf',
        reference: {'openai': 'file-abc'},
        filename: 'doc.pdf',
      );
      final json = original.toJson();
      expect(json['type'], 'file_reference');
      expect(json['reference'], {'openai': 'file-abc'});
      final decoded = ContentPart.fromJson(json);
      expect(decoded, isA<ContentPartFileReference>());
      expect((decoded as ContentPartFileReference).reference, {'openai': 'file-abc'});
    });

    test('Reasoning variant', () {
      final original = ContentPartReasoning(text: 'thinking...', signature: 'sig123');
      final json = original.toJson();
      expect(json['type'], 'reasoning');
      expect(json['text'], 'thinking...');
      expect(json['signature'], 'sig123');
      final decoded = ContentPart.fromJson(json);
      expect(decoded, isA<ContentPartReasoning>());
      expect((decoded as ContentPartReasoning).signature, 'sig123');
    });

    test('ToolCall variant', () {
      final original = ContentPartToolCall(
        toolCallId: 'call_1',
        toolName: 'get_weather',
        input: {'location': 'Tokyo'},
        providerExecuted: true,
      );
      final json = original.toJson();
      expect(json['type'], 'tool_call');
      expect(json['tool_call_id'], 'call_1');
      expect(json['tool_name'], 'get_weather');
      expect(json['provider_executed'], true);
      final decoded = ContentPart.fromJson(json);
      expect(decoded, isA<ContentPartToolCall>());
      final d = decoded as ContentPartToolCall;
      expect(d.toolCallId, 'call_1');
      expect(d.toolName, 'get_weather');
      expect(d.input, {'location': 'Tokyo'});
      expect(d.providerExecuted, true);
    });

    test('ToolResult variant — uses result, not output', () {
      final original = ContentPartToolResult(
        toolCallId: 'call_1',
        result: {'temp': 22},
        toolName: 'get_weather',
        isError: false,
        preliminary: null,
        isDynamic: null,
      );
      final json = original.toJson();
      expect(json['type'], 'tool_result');
      expect(json, contains('result'));
      expect(json, isNot(contains('output')));
      expect(json['tool_name'], 'get_weather');
      expect(json['is_error'], false);
      final decoded = ContentPart.fromJson(json);
      expect(decoded, isA<ContentPartToolResult>());
      final d = decoded as ContentPartToolResult;
      expect(d.result, {'temp': 22});
      expect(d.toolName, 'get_weather');
      expect(d.isError, false);
    });

    test('Unknown variant — forward compat', () {
      final json = {'type': 'future_variant', 'data': 'something'};
      final decoded = ContentPart.fromJson(json);
      expect(decoded, isA<ContentPartUnknown>());
      final unknown = decoded as ContentPartUnknown;
      expect(unknown.tag, 'future_variant');
      expect(unknown.data['data'], 'something');
    });
  });
}
