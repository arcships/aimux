import 'generated/ops.dart' as ops;
import 'aimux.dart';

/// A Decision handle created with provider.model using method `decision`.
final class DecisionModel {
  final Aimux client;
  final OpsHandle handle;
  DecisionModel(this.client, this.handle);

  Future<DecideResult> decide(
    DecideOptions options, {
    OpsHandle? abort,
  }) async =>
      DecideResult.fromJson((await client.call(ops.OP_DECISION_DECIDE,
          handle: handle,
          params: DecisionDecideRequest(options: options),
          abort: abort)) as Map<String, dynamic>);

  Future<Map<String, dynamic>> capabilities() async =>
      await client.call(ops.OP_DECISION_CAPABILITIES, handle: handle)
          as Map<String, dynamic>;

  Future<void> close() => handle.close();
}
