// The driver `flutter drive` needs to run an integration test in profile
// mode, which `flutter test` cannot do. See integration_test/ui_budget_test.dart.
import 'package:integration_test/integration_test_driver.dart';

Future<void> main() => integrationDriver();
