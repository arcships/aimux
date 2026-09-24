// tool_call_repair.dart — host-side tool-call repair (RFC-0035).
//
// aimux core is single-call: an unparseable tool call never fails generation,
// it comes back as a tool call with `"invalid": true`. Repair is therefore
// post-processing done here: run the normal generate/stream call with no hook,
// find the invalid calls, run the user's [RepairToolCall] on this isolate, and
// hand the reply to three pure native functions that re-validate and patch.
// Only JSON crosses the ABI — no handles, no callbacks registered with native,
// no sessions.
//
// The three functions take data only (no model handle, no tokio runtime, no
// I/O), so they are safe to call anywhere, including from a stream callback.
//
// [TypedModel] wires both helpers up; they are public so a caller driving the
// raw `Model` API can reuse the same loop.

import 'dart:async';
import 'dart:convert';
import 'dart:ffi';

import 'package:ffi/ffi.dart';

import 'aimux.dart' show promptToJson;
import 'errors.dart';
import 'types.dart';

// ─────────────────────────────────────────────────────────────────────────────
// FFI (aimux-ffi.h §"Host-side tool-call repair")
// ─────────────────────────────────────────────────────────────────────────────

// (const char *, const char *, const char *, char **out_json) → error:
// aimux_tool_call_repair_context / aimux_apply_tool_call_repair. The trailing
// string arguments are spelled nullable because `opts_json` is — it follows
// the usual NULL/empty = defaults rule. These calls take no handle, so the C
// and Dart signatures are identical and one alias serves both.
typedef _Repair3C = Pointer<Void> Function(Pointer<Utf8>, Pointer<Utf8>?,
    Pointer<Utf8>?, Pointer<Pointer<Utf8>>);

// (result_json, opts_json, tool_call_id, reply_json, char **out_json) → error.
typedef _Repair4C = Pointer<Void> Function(Pointer<Utf8>, Pointer<Utf8>?,
    Pointer<Utf8>, Pointer<Utf8>, Pointer<Pointer<Utf8>>);

final DynamicLibrary _lib = openAimuxLibrary();

final _repairContext =
    _lib.lookupFunction<_Repair3C, _Repair3C>('aimux_tool_call_repair_context');
final _applyRepair =
    _lib.lookupFunction<_Repair3C, _Repair3C>('aimux_apply_tool_call_repair');
final _applyRepairToResult = _lib.lookupFunction<_Repair4C, _Repair4C>(
    'aimux_apply_tool_call_repair_to_result');

/// `(required, optional, optional) → json` — the shape both three-argument
/// exports share.
String _call3(_Repair3C fn, String a, String? b, String? c, String context) {
  return withUtf8(a, (pa) {
    final pb = toCStringOrNull(b);
    final pc = toCStringOrNull(c);
    try {
      return takeString((out) => fn(pa, pb, pc, out), context);
    } finally {
      if (pb != nullptr) calloc.free(pb);
      if (pc != nullptr) calloc.free(pc);
    }
  });
}

/// `(result_json, opts_json?, tool_call_id, reply_json) → json`.
String _call4(_Repair4C fn, String a, String? b, String c, String d,
    String context) {
  return withUtf8(a, (pa) {
    return withUtf8(c, (pc) {
      return withUtf8(d, (pd) {
        final pb = toCStringOrNull(b);
        try {
          return takeString((out) => fn(pa, pb, pc, pd, out), context);
        } finally {
          if (pb != nullptr) calloc.free(pb);
        }
      });
    });
  });
}

// ─────────────────────────────────────────────────────────────────────────────
// One call: context → user hook → reply
// ─────────────────────────────────────────────────────────────────────────────

/// The repair argument for one invalid [call], or null when the call was made
/// without a tool set — the AI SDK never repairs such a call, and the core
/// decides it, so null means "skip this one".
ToolCallRepairContext? _contextFor(
    Map<String, dynamic> call, String promptJson, String? optsJson) {
  final json = jsonDecode(_call3(_repairContext, jsonEncode(call), promptJson,
      optsJson, 'tool_call_repair_context'));
  if (json == null) return null;
  return ToolCallRepairContext.fromJson(json as Map<String, dynamic>);
}

Map<String, dynamic> _replyFor(RawToolCall? replacement) => replacement == null
    ? const {'type': 'unchanged'}
    : {'type': 'repaired', 'tool_call': replacement.toJson()};

/// A thrown object has no typed counterpart in the core, so it becomes
/// `failed` — the core turns that into `ToolCallRepair{cause: Other(message)}`.
///
/// The message is the thrown object's `toString()`, which the caller does not
/// control the way it controls a replacement call — so an unpaired surrogate
/// in it is replaced rather than rejected by [encodeJson]: the UTF-8 round
/// trip writes U+FFFD for one.
Map<String, dynamic> _failedReply(Object error) =>
    {'type': 'failed', 'message': utf8.decode(utf8.encode('$error'))};

/// The synchronous entry points cannot await, so a [Future] return is a
/// programming error rather than a repair failure and must not be swallowed
/// into a `failed` reply.
Never _asyncHookInSyncCall() => throw StateError(
    'repairToolCall returned a Future, but this generate call is synchronous '
    '— use a synchronous hook here, or repair on the streamText path');

// ─────────────────────────────────────────────────────────────────────────────
// Non-streaming: patch a whole result document
// ─────────────────────────────────────────────────────────────────────────────

/// Repair every invalid tool call in a `generate_text` / `generate_object` /
/// aggregated-stream [result], returning the patched document.
///
/// Returns [result] unchanged when `options.repairToolCall` is null. [prompt]
/// and [options] must be the ones the result was generated with: the core
/// derives the tool set, messages and instructions from them.
///
/// Call this on the raw result map *before* decoding it into typed classes —
/// both `tool_calls` and the matching `response_messages` part are rewritten.
Map<String, dynamic> repairResultToolCalls(
  Map<String, dynamic> result,
  Object prompt,
  GenerateTextOptions? options,
) {
  final repair = options?.repairToolCall;
  if (repair == null) return result;

  final promptJson = promptToJson(prompt);
  final optsJson = encodeJson(options!.toJson(), 'options');

  var current = result;
  // Snapshot the invalid calls first: each patch rewrites the document, and a
  // repair may even rename its call, but the ids collected here stay the ones
  // still to be offered. One attempt per call (AI SDK).
  for (final call in _invalidToolCalls(current)) {
    final context = _contextFor(call, promptJson, optsJson);
    if (context == null) continue;

    // Only what the hook itself throws becomes a `failed` reply; the
    // async-hook check below stays a real error, so it runs outside the catch.
    Map<String, dynamic>? reply;
    FutureOr<RawToolCall?> replacement;
    try {
      replacement = repair(context);
    } catch (e) {
      replacement = null;
      reply = _failedReply(e);
    }
    if (reply == null) {
      if (replacement is Future) _asyncHookInSyncCall();
      reply = _replyFor(replacement);
    }

    current = jsonDecode(_call4(
        _applyRepairToResult,
        jsonEncode(current),
        optsJson,
        call['tool_call_id'] as String,
        encodeJson(reply, 'repairToolCall reply'),
        'apply_tool_call_repair_to_result')) as Map<String, dynamic>;
  }
  return current;
}

/// The invalid calls of a result document. `generate_object` nests the
/// `generate_text` result under `raw`.
List<Map<String, dynamic>> _invalidToolCalls(Map<String, dynamic> result) {
  final calls = result['tool_calls'] ??
      (result['raw'] as Map<String, dynamic>?)?['tool_calls'];
  if (calls is! List) return const [];
  return calls
      .whereType<Map<String, dynamic>>()
      .where((c) => c['invalid'] == true)
      .toList();
}

// ─────────────────────────────────────────────────────────────────────────────
// Streaming: patch the tool-call part in flight
// ─────────────────────────────────────────────────────────────────────────────

/// Replace every invalid `ToolCall` part of [parts] with its repaired version.
///
/// Returns [parts] unchanged when `options.repairToolCall` is null. Every other
/// part — tool-input deltas included — passes through untouched and in order
/// (the AI SDK never holds argument deltas back), which `asyncMap` guarantees
/// by awaiting one element before pulling the next.
Stream<Map<String, dynamic>> repairStreamToolCalls(
  Stream<Map<String, dynamic>> parts,
  Object prompt,
  GenerateTextOptions? options,
) {
  final repair = options?.repairToolCall;
  if (repair == null) return parts;

  final promptJson = promptToJson(prompt);
  final optsJson = encodeJson(options!.toJson(), 'options');

  return parts.asyncMap((part) async {
    // Externally tagged: {"ToolCall": {...}}.
    final call = part['ToolCall'];
    if (call is! Map<String, dynamic> || call['invalid'] != true) return part;

    final context = _contextFor(call, promptJson, optsJson);
    if (context == null) return part;

    Map<String, dynamic> reply;
    try {
      reply = _replyFor(await repair(context));
    } catch (e) {
      reply = _failedReply(e);
    }

    final patched = _call3(_applyRepair, jsonEncode(call), optsJson,
        encodeJson(reply, 'repairToolCall reply'), 'apply_tool_call_repair');
    return {'ToolCall': jsonDecode(patched)};
  });
}
