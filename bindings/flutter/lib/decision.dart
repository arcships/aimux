import 'dart:convert';
import 'dart:ffi';
import 'package:ffi/ffi.dart';
import 'errors.dart';

typedef _New = Pointer<Void> Function(Pointer<Utf8>, Pointer<Utf8>,
    Pointer<Utf8>, Pointer<Utf8>, Pointer<Uint64>);
typedef _DecideC = Pointer<Void> Function(
    Uint64, Pointer<Utf8>, Uint64, Pointer<Pointer<Utf8>>);
typedef _DecideD = Pointer<Void> Function(
    int, Pointer<Utf8>, int, Pointer<Pointer<Utf8>>);
typedef _CapabilitiesC = Pointer<Void> Function(Uint64, Pointer<Pointer<Utf8>>);
typedef _CapabilitiesD = Pointer<Void> Function(int, Pointer<Pointer<Utf8>>);

typedef _ProviderDecisionC = Pointer<Void> Function(Uint64, Pointer<Utf8>, Pointer<Uint64>);
typedef _ProviderDecisionD = Pointer<Void> Function(int, Pointer<Utf8>, Pointer<Uint64>);

final _lib = openAimuxLibrary();
final _providerDecision = _lib.lookupFunction<_ProviderDecisionC, _ProviderDecisionD>(
    'aimux_provider_decision_model');
final _newDecision = _lib.lookupFunction<_New, _New>(
    'aimux_jev_decision_new_with_probability_source');
final _decide = _lib.lookupFunction<_DecideC, _DecideD>('aimux_decide_with_abort');
final _capabilities = _lib.lookupFunction<_CapabilitiesC, _CapabilitiesD>(
    'aimux_decision_capabilities');

/// Native decisions using the shared core JSON contract.
class DecisionModel {
  int _handle;
  DecisionModel._(this._handle);

  /// Native interop used by ProviderHandle.decisionModel; owns the returned handle.
  factory DecisionModel.fromProviderHandle(int providerHandle, String modelId) {
    return withUtf8(modelId, (id) => DecisionModel._(takeHandle(
        (out) => _providerDecision(providerHandle, id, out), 'decision model')));
  }

  /// Endpoint is a complete POST URL; null uses the official API.
  factory DecisionModel.jev(String apiKey, String modelId,
      {String? endpoint, String? probabilitySource}) {
    Pointer<Utf8> key = nullptr, id = nullptr, url = nullptr, source = nullptr;
    try {
      key = toCString(apiKey);
      id = toCString(modelId);
      url = toCStringOrNull(endpoint);
      source = toCStringOrNull(probabilitySource);
      return DecisionModel._(takeHandle(
          (out) => _newDecision(key, id, url, source, out), 'jev decision'));
    } finally {
      calloc.free(key);
      calloc.free(id);
      calloc.free(url);
      calloc.free(source);
    }
  }

  /// Raw JSON API. Optional abort handle uses the existing C ABI lifecycle.
  String decideJson(String options, {int abortHandle = 0}) {
    _checkOpen();
    checkJson(options, 'options');
    return withUtf8(options, (ptr) => takeString(
        (out) => _decide(_handle, ptr, abortHandle, out), 'decide'));
  }

  /// Dart maps/lists preserve native structured instructions and criteria.
  Map<String, dynamic> decide(Map<String, dynamic> options,
          {int abortHandle = 0}) =>
      (jsonDecode(decideJson(encodeJson(options, 'options'),
          abortHandle: abortHandle)) as Map).cast<String, dynamic>();

  /// Query capabilities and rounding precision without HTTP.
  Map<String, dynamic> capabilities() {
    _checkOpen();
    return (jsonDecode(takeString(
        (out) => _capabilities(_handle, out), 'decision capabilities')) as Map)
        .cast<String, dynamic>();
  }

  void close() {
    if (_handle != 0) {
      aimuxDropHandle(_handle);
      _handle = 0;
    }
  }

  void _checkOpen() {
    if (_handle == 0) throw StateError('DecisionModel is closed');
  }
}
