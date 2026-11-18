import 'package:aimux/aimux.dart';
import 'package:aimux/generated/ops.dart' as ops;
import 'package:flutter/widgets.dart';

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
    return WidgetsApp(
      title: 'aimux demo',
      color: const Color(0xFF3F51B5),
      pageRouteBuilder: <T>(RouteSettings settings, WidgetBuilder builder) =>
          PageRouteBuilder<T>(
            settings: settings,
            pageBuilder: (
              BuildContext context,
              Animation<double> animation,
              Animation<double> secondaryAnimation,
            ) =>
                builder(context),
          ),
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
    return Directionality(
      textDirection: TextDirection.ltr,
      child: ColoredBox(
        color: const Color(0xFFFFFFFF),
        child: SafeArea(
          child: DefaultTextStyle(
            style: const TextStyle(color: Color(0xFF1F2937), fontSize: 15),
            child: Padding(
              padding: const EdgeInsets.all(16),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  const Text(
                    'aimux demo',
                    style: TextStyle(fontSize: 20, fontWeight: FontWeight.w600),
                  ),
                  const SizedBox(height: 16),
                  GestureDetector(
                    onTap: _busy ? null : _describe,
                    child: DecoratedBox(
                      decoration: BoxDecoration(
                        color: Color(_busy ? 0xFF9CA3AF : 0xFF3F51B5),
                        borderRadius: const BorderRadius.all(Radius.circular(8)),
                      ),
                      child: const Padding(
                        padding: EdgeInsets.symmetric(horizontal: 16, vertical: 12),
                        child: Center(
                          child: Text(
                            'Describe runtime',
                            style: TextStyle(
                              color: Color(0xFFFFFFFF),
                              fontWeight: FontWeight.w600,
                            ),
                          ),
                        ),
                      ),
                    ),
                  ),
                  const SizedBox(height: 16),
                  Expanded(
                    child: SingleChildScrollView(child: Text(_output)),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
