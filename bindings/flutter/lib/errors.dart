/// The lossless error envelope. Unknown payload variants are preserved.
final class OpsException implements Exception {
  final Map<String, dynamic> envelope;

  OpsException.fromJson(Map<String, dynamic> json)
      : envelope = Map.unmodifiable(json) {
    if (json['kind'] is! String ||
        json['code'] is! int ||
        json['message'] is! String ||
        json['error'] is! Map ||
        (json.containsKey('retryAfterMs') &&
            (json['retryAfterMs'] is! int ||
                (json['retryAfterMs'] as int) < 0))) {
      throw const FormatException('Invalid aimux error envelope');
    }
  }

  String get kind => envelope['kind'] as String;
  int get code => envelope['code'] as int;
  String get message => envelope['message'] as String;
  Map<String, dynamic> get error =>
      Map<String, dynamic>.unmodifiable(envelope['error'] as Map);
  int? get retryAfterMs => envelope['retryAfterMs'] as int?;

  /// The payload's cause: a nested AI SDK error (with `name`) or an
  /// underlying OS, TLS or parser error as `{message, cause?}`.
  Map<String, dynamic>? get cause {
    final cause = (envelope['error'] as Map)['cause'];
    return cause is Map ? Map<String, dynamic>.unmodifiable(cause) : null;
  }
  Map<String, dynamic> toJson() => Map.of(envelope);

  @override
  String toString() => 'OpsException($kind/$code): $message';
}

OpsException bindingFailure(String message) => OpsException.fromJson({
      'kind': 'binding',
      'code': 206,
      'message': message,
      'error': {'name': 'CallbackFailure', 'message': message},
    });

OpsException resourceLimit(String message) => OpsException.fromJson({
      'kind': 'protocol',
      'code': 305,
      'message': message,
      'error': {'name': 'ResourceLimit', 'message': message},
    });
