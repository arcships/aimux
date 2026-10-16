import 'dart:typed_data';

import 'generated/ops.dart' as ops;
import 'aimux.dart';

/// A realtime transcription operation created through transcription.start.
/// Model and modality calls use Aimux.call and the manifest's canonical DTOs.
final class TranscriptionOperation {
  final Aimux client;
  final OpsHandle handle;
  TranscriptionOperation(this.client, this.handle);

  Future<void> push(Uint8List audio) => client.push(handle, audio);
  Future<Object?> inputDone() =>
      client.call(ops.OP_OPERATION_INPUT_DONE, handle: handle);
  Future<OperationPollResult> next({int timeoutMs = 0}) =>
      client.next(handle, timeoutMs: timeoutMs);
  Future<Object?> status() =>
      client.call(ops.OP_OPERATION_STATUS, handle: handle);
  Future<Object?> cancel() =>
      client.call(ops.OP_OPERATION_CANCEL, handle: handle);
  Future<void> close() => handle.close();
}
