import 'dart:io';

import '../lib/src/generated/dto_vectors.dart';
import '../lib/src/generated/types.dart';

void main() {
  final directory = Directory('contract-tests/dto');
  final index =
      dtoJsonDecode(File('${directory.path}/index.json').readAsStringSync())
          as List;
  for (final entry in index.cast<Map>()) {
    final file = entry['file'] as String;
    try {
      final value =
          dtoJsonDecode(File('${directory.path}/$file').readAsStringSync());
      print(dtoJsonEncode({
        'file': file,
        'ok': true,
        'value': roundTripDto(entry['type'] as String, value)
      }));
    } catch (error) {
      print(dtoJsonEncode(
          {'file': file, 'ok': false, 'error': error.toString()}));
    }
  }
}
