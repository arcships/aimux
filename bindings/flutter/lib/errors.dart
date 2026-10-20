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
