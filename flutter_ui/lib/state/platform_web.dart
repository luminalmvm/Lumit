// What platform.dart answers, for the browser build. A page is on no
// operating system Lumit has code for, so every question about one is no.

abstract final class Platform {
  static const bool isWindows = false;
  static const bool isMacOS = false;
  static const bool isLinux = false;
  static const bool isAndroid = false;
  static const bool isIOS = false;
  static const bool isFuchsia = false;
  static const String operatingSystem = 'web';
  static const String pathSeparator = '/';
  // Where files_web.dart keeps the settings: they go under `HOME/.config`.
  static const Map<String, String> environment = {'HOME': '/home'};
  static const String resolvedExecutable = '';
  static const String executable = '';
  static const String localHostname = '';
  static const int numberOfProcessors = 1;
  static const String localeName = 'en';
  static const String version = '';
}
