import 'package:flutter_test/flutter_test.dart';

import 'package:aimux_example/main.dart';

void main() {
  testWidgets('demo app renders', (WidgetTester tester) async {
    await tester.pumpWidget(const AimuxDemoApp());
    expect(find.text('aimux demo'), findsOneWidget);
    expect(find.text('Generate'), findsOneWidget);
  });
}
