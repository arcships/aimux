import 'package:flutter/material.dart';
import 'package:aimux/aimux.dart';
import 'package:aimux/generated/ops.dart' as ops;

void main() {
  runApp(const AimuxDemoApp());
}

/// Minimal demo of the aimux Flutter plugin.
///
/// The native core (aimux-ffi) ships inside the package — no extra setup.
/// Provider/model request DTOs follow the loaded ops manifest.
class AimuxDemoApp extends StatelessWidget {
  const AimuxDemoApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'aimux demo',
      theme: ThemeData(colorSchemeSeed: Colors.indigo, useMaterial3: true),
      home: const HomePage(),
    );
  }
}

class HomePage extends StatefulWidget {
  const HomePage({super.key});

  @override
  State<HomePage> createState() => _HomePageState();
}

class _HomePageState extends State<HomePage> {
  String _output = 'Press "Describe runtime" to inspect native capabilities.';
  bool _busy = false;

  Future<void> _describe() async {
    setState(() {
      _busy = true;
      _output = 'Loading runtime…';
    });
    try {
      final client = await Aimux.open(requiredOps: [ops.OP_RUNTIME_DESCRIBE]);
      final result = await client.call(ops.OP_RUNTIME_DESCRIBE);
      if (mounted) setState(() => _output = result.toString());
    } catch (e) {
      if (mounted) setState(() => _output = 'Error: $e');
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('aimux demo')),
      body: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            FilledButton(
              onPressed: _busy ? null : _describe,
              child: const Text('Describe runtime'),
            ),
            const SizedBox(height: 16),
            Expanded(
              child: SingleChildScrollView(child: SelectableText(_output)),
            ),
          ],
        ),
      ),
    );
  }
}
