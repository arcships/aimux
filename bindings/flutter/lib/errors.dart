import 'dart:convert';

import 'src/generated/types.dart';

/// The native error envelope presented as a Dart exception.
final class OpsException implements Exception {
  final ErrorEnvelope envelope;
  const OpsException(this.envelope);
  factory OpsException.fromJson(Map<String, dynamic> json) =>
      OpsException(ErrorEnvelope.fromJson(json));

  String get kind => envelope.kind.wireValue;
  int get code => envelope.code;
  String get message => envelope.message;
  Object? get error => envelope.error;
  Map<String, dynamic> toJson() => envelope.toJson();

  /// The payload's cause: a nested AI SDK error
  /// ([ErrorCauseBranchAiMuxError]) or an underlying OS, TLS or parser error
  /// ([ErrorCauseBranchSourceError], `{message, cause?}`). A retry error's
  /// cause is its last attempt, the AI SDK `lastError`. The raw JSON stays in
  /// `error['cause']`.
  ErrorCause? get cause {
    final error = envelope.error;
    if (error is! Map) return null;
    var cause = error['cause'];
    final errors = error['errors'];
    if (cause == null &&
        error['name'] == 'AI_RetryError' &&
        errors is List &&
        errors.isNotEmpty) {
      cause = errors.last;
    }
    if (cause is! Map) return null;
    try {
      // A JSON round trip gives nested maps the `Map<String, dynamic>` type
      // the generated decoders expect, whatever map type the envelope holds.
      return ErrorCause.fromJson(jsonDecode(jsonEncode(cause)));
    } catch (_) {
      return null;
    }
  }

  @override
  String toString() => 'OpsException($kind/$code): $message';
}

OpsException bindingFailure(String message) => OpsException(ErrorEnvelope(
      kind: ErrorDomain.binding,
      code: 206,
      message: message,
      error: {'name': 'CallbackFailure', 'message': message},
    ));

OpsException resourceLimit(String message) => OpsException(ErrorEnvelope(
      kind: ErrorDomain.protocol,
      code: 305,
      message: message,
      error: {'name': 'ResourceLimit', 'message': message},
    ));
