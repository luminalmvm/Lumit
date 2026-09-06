// Opening a web page in the machine's own browser.
//
// In plain terms: Lumit has no browser of its own and is never going to grow
// one. Help ▸ Lumit help hands a web address to the desktop and lets whatever
// the user already reads the web with open it — the same thing the update
// download does when it reveals a file (state/updates.dart), through the same
// per-platform launcher.
//
// **Nothing here goes through a shell.** `Process.start` is given a program and
// a list of arguments, so a URL is one argument whatever punctuation it
// contains; there is no command line for it to be part of. The scheme is
// checked anyway, so a `file:` or a `javascript:` cannot be handed to the
// desktop by a caller that meant a web page.

import 'dart:io';

/// The documentation site, and the page that walks a newcomer through their
/// first composition. Both are real pages on `web-docs/`; the sidebar
/// sections themselves have no index page, so a link must name a page.
const String lumitDocsUrl = 'https://docs.lumitlab.com/';
const String lumitGuidesUrl =
    'https://docs.lumitlab.com/start/first-composition/';

/// What shipped in each version — the welcome screen's *What's new*. It is on
/// the main site rather than the documentation one, because the release notes
/// are `web/src/pages/releases/`, not a docs page.
const String lumitReleasesUrl = 'https://lumitlab.com/releases/';

/// How Lumit opens a web page, and the seam a test replaces.
///
/// A top-level function rather than a parameter threaded through every caller:
/// this is the desktop, not a collaborator, and every caller wants the same
/// one. A suite that must not launch a browser swaps it and puts it back.
Future<bool> Function(String url) openExternalLink = launchInDefaultBrowser;

/// Hand [url] to the desktop. Answers whether the launcher started, not
/// whether a page ever appeared — that is the browser's business and there is
/// nothing to wait for.
///
/// Never throws: a machine with no browser registered, or no `xdg-open`, has
/// not done anything wrong, and an editor that fell over because a help link
/// could not be followed would be absurd. The caller says so in the status
/// line instead.
Future<bool> launchInDefaultBrowser(String url) async {
  if (!_isWebAddress(url)) return false;
  try {
    switch (Platform.operatingSystem) {
      case 'windows':
        // What Windows itself uses to follow a link, and the one launcher that
        // opens no console window on the way — `cmd /c start` flashes one.
        await Process.start(
          'rundll32',
          ['url.dll,FileProtocolHandler', url],
          mode: ProcessStartMode.detached,
        );
      case 'macos':
        await Process.start('open', [url], mode: ProcessStartMode.detached);
      default:
        await Process.start('xdg-open', [url], mode: ProcessStartMode.detached);
    }
    return true;
  } catch (_) {
    return false;
  }
}

/// Whether this is a web address and not something else wearing a URL's shape.
bool _isWebAddress(String url) {
  final parsed = Uri.tryParse(url);
  return parsed != null &&
      (parsed.scheme == 'https' || parsed.scheme == 'http') &&
      parsed.host.isNotEmpty;
}
