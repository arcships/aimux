import 'dart:convert';
import 'dart:io';

import '../lib/src/generated/dto_vectors.dart';

void main() {
  final directory = Directory('contract-tests/dto');
  final index =
      jsonDecode(File('${directory.path}/index.json').readAsStringSync())
          as List;
  for (final entry in index.cast<Map>()) {
    final file = entry['file'] as String;
    try {
      final value =
          jsonDecode(File('${directory.path}/$file').readAsStringSync());
      print(jsonEncode({
        'file': file,
        'ok': true,
        'value': roundTripDto(entry['type'] as String, value)
      }));
    } catch (error) {
      print(jsonEncode({'file': file, 'ok': false, 'error': error.toString()}));
    }
  }
}
