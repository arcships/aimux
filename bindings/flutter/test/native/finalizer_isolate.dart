import 'dart:isolate';
import 'package:aimux/aimux.dart';

// spawnUri creates a separate isolate group; NativeFinalizer must run by its
// orderly shutdown. The fixture's native drop counters are process-wide.
void main(List<String> args, SendPort ready) async {
  final client = await Aimux.open(libraryPath: args[0], bridgePath: args[0]);
  final result = await client.call('test.handle') as Map;
  final handle = client.adopt(result['handle'] as String);
  final id = handle.wireId;
  if (args[1] == 'close') await handle.close();
  ready.send(id);
}
