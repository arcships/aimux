import 'dart:convert';
import 'dart:ffi';
import 'dart:io';

import 'package:aimux/errors.dart';
import 'package:aimux/tool_call_repair.dart';
import 'package:aimux/types.dart';
import 'package:ffi/ffi.dart';
import 'package:test/test.dart';

typedef _Repair3 = Pointer<Void> Function(Pointer<Utf8>, Pointer<Utf8>?,
    Pointer<Utf8>?, Pointer<Pointer<Utf8>>);
typedef _Repair4 = Pointer<Void> Function(Pointer<Utf8>, Pointer<Utf8>?,
    Pointer<Utf8>, Pointer<Utf8>, Pointer<Pointer<Utf8>>);

final _lib = openAimuxLibrary();
final _context = _lib.lookupFunction<_Repair3, _Repair3>(
    'aimux_tool_call_repair_context');
final _apply = _lib.lookupFunction<_Repair3, _Repair3>(
    'aimux_apply_tool_call_repair');
final _patch = _lib.lookupFunction<_Repair4, _Repair4>(
    'aimux_apply_tool_call_repair_to_result');
final List<Map<String, dynamic>> _cases =
    (jsonDecode(_fixture().readAsStringSync())['cases'] as List)
        .cast<Map<String, dynamic>>();

void main() {
  test('replays every shared contract fixture case', () {
    expect(_cases, isNotEmpty);
    for (final fixture in _cases) {
      final input = fixture['input'] as Map<String, dynamic>;
      Object? run() => jsonDecode(_invoke(fixture['function'] as String, input));
      if (fixture.containsKey('expected_error')) {
        expect(run, throwsA(isA<AimuxException>().having(
            (e) => e.code, 'code', AimuxErrorCode.invalidArgument)),
            reason: fixture['name'] as String);
      } else {
        expect(run(), equals(fixture['expected']),
            reason: fixture['name'] as String);
      }
    }
  });

  test('public result repair receives raw input and patches transcript', () {
    final fixture = _named('patch_result');
    final input = fixture['input'] as Map<String, dynamic>;
    String? rawInput;
    final result = repairResultToolCalls(
      jsonDecode(jsonEncode(input['result'])) as Map<String, dynamic>,
      'weather in Singapore?',
      _options(input['opts'] as Map<String, dynamic>, (context) {
        rawInput = context.toolCall.input;
        return RawToolCall.fromJson(
            input['reply']['tool_call'] as Map<String, dynamic>);
      }),
    );
    expect(rawInput, '{"town":"Singapore"}');
    expect(result['tool_calls'], fixture['expected']['tool_calls']);
    expect(result['response_messages'], fixture['expected']['response_messages']);
  });

  test('public stream awaits repair and preserves input deltas', () async {
    final fixture = _named('reply_repaired_validates');
    final input = fixture['input'] as Map<String, dynamic>;
    final deltas = [
      {'ToolInputDelta': {'id': 'call-1', 'delta': '{"town":'}},
      {'ToolInputDelta': {'id': 'call-1', 'delta': '"Singapore"}'}},
    ];
    final parts = [...deltas, {'ToolCall': input['tool_call']}];
    final out = await repairStreamToolCalls(
      Stream.fromIterable(parts),
      'weather in Singapore?',
      _options(input['opts'] as Map<String, dynamic>, (_) async =>
          RawToolCall.fromJson(input['reply']['tool_call'] as Map<String, dynamic>)),
    ).toList();
    expect(out.take(2), equals(deltas));
    expect(out.last['ToolCall'], equals(fixture['expected']));
  });

  test('synchronous repair rejects an async hook', () {
    final fixture = _named('reply_unchanged');
    final input = fixture['input'] as Map<String, dynamic>;
    expect(
      () => repairResultToolCalls(_result(input['tool_call'] as Map<String, dynamic>),
          'weather?', _options(input['opts'] as Map<String, dynamic>, (_) async => null)),
      throwsStateError,
    );
  });

  // The four host-loop branches the shared fixture cannot reach: whether the
  // hook runs at all, and what a throw or a null return leaves behind.
  test('host loop reports a failed repair and skips calls it must not offer',
      () {
    int hookCalls = 0;
    RawToolCall? counting(ToolCallRepairContext context) {
      hookCalls++;
      return null;
    }

    // 1. the hook throws → invalid, wrapped in ToolCallRepair{cause: Other}.
    final Map<String, dynamic> patch =
        _named('patch_result')['input'] as Map<String, dynamic>;
    final Map<String, dynamic> invalidCall =
        _firstCall(patch['result'] as Map<String, dynamic>);
    final Map<String, dynamic> failed = repairResultToolCalls(
      _result(_copy(invalidCall)),
      'weather in Singapore?',
      // A bare String, not an Exception: the core records the thrown object's
      // toString(), and an Exception would prefix it with "Exception: ".
      _options(patch['opts'] as Map<String, dynamic>, (_) {
        hookCalls++;
        throw 'repair model unavailable';
      }),
    );
    final Map<String, dynamic> failedCall = _firstCall(failed);
    final Map<String, dynamic> wrapped =
        (failedCall['error'] as Map<String, dynamic>)['ToolCallRepair']
            as Map<String, dynamic>;
    expect(failedCall['invalid'], isTrue);
    expect(wrapped['cause'], {'Other': 'repair model unavailable'});
    expect(wrapped['original_error'], invalidCall['error']);

    // 2. the hook returns null → the ORIGINAL error survives, unwrapped.
    final Map<String, dynamic> kept = repairResultToolCalls(
      _result(_copy(invalidCall)),
      'weather in Singapore?',
      _options(patch['opts'] as Map<String, dynamic>, counting),
    );
    expect(_firstCall(kept)['invalid'], isTrue);
    expect(_firstCall(kept)['error'], invalidCall['error']);
    expect(hookCalls, 2);

    // 3. a valid call is never offered to the hook.
    final Map<String, dynamic> valid =
        _named('reject_valid_tool_call')['input'] as Map<String, dynamic>;
    final Map<String, dynamic> untouched = repairResultToolCalls(
      _result(_copy(valid['tool_call'] as Map<String, dynamic>)),
      'weather in Singapore?',
      _options(valid['opts'] as Map<String, dynamic>, counting),
    );
    expect(_firstCall(untouched)['invalid'], isNull);

    // 4. no tool set → the native context answers null and the host skips.
    final Map<String, dynamic> noTools =
        _named('context_without_tools')['input'] as Map<String, dynamic>;
    final Map<String, dynamic> skipped = repairResultToolCalls(
      _result(_copy(noTools['tool_call'] as Map<String, dynamic>)),
      'weather in Singapore?',
      _options(noTools['opts'] as Map<String, dynamic>, counting),
    );
    expect(_firstCall(skipped)['invalid'], isTrue);
    expect(hookCalls, 2);
  });
}

String _invoke(String function, Map<String, dynamic> input) {
  final opts = input['opts'] == null ? null : jsonEncode(input['opts']);
  switch (function) {
    case 'tool_call_repair_context':
      return _call3(_context, jsonEncode(input['tool_call']),
          jsonEncode(input['prompt']), opts);
    case 'apply_tool_call_repair':
      return _call3(_apply, jsonEncode(input['tool_call']), opts,
          jsonEncode(input['reply']));
    case 'apply_tool_call_repair_to_result':
      return _call4(jsonEncode(input['result']), opts,
          input['tool_call_id'] as String, jsonEncode(input['reply']));
    default:
      throw StateError('unknown fixture function: $function');
  }
}

String _call3(_Repair3 function, String a, String? b, String? c) =>
    withUtf8(a, (pa) {
      final pb = toCStringOrNull(b), pc = toCStringOrNull(c);
      try {
        return takeString((out) => function(pa, pb, pc, out), 'fixture');
      } finally {
        if (pb != nullptr) calloc.free(pb);
        if (pc != nullptr) calloc.free(pc);
      }
    });

String _call4(String a, String? b, String c, String d) => withUtf8(a, (pa) =>
    withUtf8(c, (pc) => withUtf8(d, (pd) {
      final pb = toCStringOrNull(b);
      try {
        return takeString((out) => _patch(pa, pb, pc, pd, out), 'fixture');
      } finally {
        if (pb != nullptr) calloc.free(pb);
      }
    })));

GenerateTextOptions _options(
    Map<String, dynamic> json, RepairToolCall repair) {
  final base = GenerateTextOptions.fromJson(json);
  return GenerateTextOptions(tools: base.tools, instructions: base.instructions,
      repairToolCall: repair);
}

Map<String, dynamic> _result(Map<String, dynamic> call) => {
  'text': '', 'tool_calls': [call],
  'response_messages': [{'role': 'assistant', 'content': [
    {'type': 'tool_call', 'tool_call_id': call['tool_call_id'],
      'tool_name': call['tool_name'], 'input': call['input']}
  ]}]
};

Map<String, dynamic> _firstCall(Map<String, dynamic> result) =>
    (result['tool_calls'] as List).first as Map<String, dynamic>;

Map<String, dynamic> _copy(Map<String, dynamic> json) =>
    jsonDecode(jsonEncode(json)) as Map<String, dynamic>;

Map<String, dynamic> _named(String name) =>
    _cases.singleWhere((fixture) => fixture['name'] == name);

File _fixture() => [
  '../../contract-tests/fixtures/tool-call-repair.json',
  'contract-tests/fixtures/tool-call-repair.json',
  '../contract-tests/fixtures/tool-call-repair.json',
].map(File.new).firstWhere((file) => file.existsSync());
