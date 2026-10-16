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
      DecideResult.fromJson((await client.call('decision.decide',
          handle: handle,
          params: DecisionDecideRequest(options: options),
          abort: abort)) as Map<String, dynamic>);

  Future<Map<String, dynamic>> capabilities() async =>
      await client.call('decision.capabilities', handle: handle)
          as Map<String, dynamic>;

  Future<void> close() => handle.close();
}
