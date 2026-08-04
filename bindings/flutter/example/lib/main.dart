import 'package:flutter/material.dart';
import 'package:aimux/aimux.dart';

void main() {
  runApp(const AimuxDemoApp());
}

/// Minimal demo of the aimux Flutter plugin.
///
/// The native core (aimux-ffi) ships inside the package — no extra setup.
/// Replace the fake credentials below with real ones to try a live call,
/// or point [Model.openaiWithBase] at a local mock server (e.g. the
/// contract-test server in this repository).
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
  final _apiKeyController = TextEditingController(text: 'sk-test-fake-key');
  final _modelController = TextEditingController(text: 'gpt-4o-mini');
  String _output = 'Press "Generate" to call the model.';
  bool _busy = false;

  @override
  void dispose() {
    _apiKeyController.dispose();
    _modelController.dispose();
    super.dispose();
  }

  Future<void> _generate() async {
    setState(() {
      _busy = true;
      _output = 'Generating…';
    });
    try {
      // Local mock by default — swap the base URL for a real provider.
      final model = Model.openai(
        _apiKeyController.text,
        _modelController.text,
        baseUrl: 'http://localhost:3000',
      );
      try {
        final result = model.generateText('What is Rust?');
        setState(() => _output = result.toString());
      } finally {
        model.close();
      }
    } catch (e) {
      setState(() => _output = 'Error: $e');
    } finally {
      setState(() => _busy = false);
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
            TextField(
              controller: _apiKeyController,
              decoration: const InputDecoration(labelText: 'API key'),
            ),
            const SizedBox(height: 8),
            TextField(
              controller: _modelController,
              decoration: const InputDecoration(labelText: 'Model ID'),
            ),
            const SizedBox(height: 16),
            FilledButton(
              onPressed: _busy ? null : _generate,
              child: const Text('Generate'),
            ),
            const SizedBox(height: 16),
            Expanded(
              child: SingleChildScrollView(
                child: SelectableText(_output),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
