; Lumit Windows installer (Inno Setup). Build with:
;   packaging/windows/build-installer.ps1
; or by hand: flutter build windows --release (from flutter_ui/), then
;   iscc packaging\windows\lumit.iss
;
; Registers the .lum, .lumfx and .lumtheme associations with their document icons
; (assets/brand) and an open command; Lumit itself reads the document
; path from the command line (projectPathFromArgs in flutter_ui/lib/main.dart).

; Keep in step with flutter_ui/pubspec.yaml `version:` when cutting a release.
; The release workflow overrides this from the tag (iscc /DMyAppVersion=...).
#ifndef MyAppVersion
#define MyAppVersion "0.1.0"
#endif
#define MyAppExe "lumit.exe"

[Setup]
AppId={{8B6F1C6A-9E4B-4C7D-B1A4-6C1E5D2F7A31}
AppName=Lumit
AppVersion={#MyAppVersion}
AppPublisher=Lumit
AppPublisherURL=https://github.com/luminalmvm/Lumit
; Per user, not per machine, the way Chrome and VS Code install. {localappdata}
; belongs to the person running it, so neither the first install nor an update
; needs an administrator: an update downloads this installer again and runs it
; silently over the folder it installed into. `PrivilegesRequired=lowest` means
; no UAC prompt either way.
PrivilegesRequired=lowest
DefaultDirName={localappdata}\Programs\Lumit
; An existing installation keeps its folder, wherever a previous version put it
; — including the old {autopf} one, which simply carries on being updated by
; this installer rather than in place.
UsePreviousAppDir=yes
DefaultGroupName=Lumit
LicenseFile=..\..\LICENSE
OutputDir=dist
OutputBaseFilename=lumit-{#MyAppVersion}-windows-x64-setup
SetupIconFile=..\..\flutter_ui\windows\runner\resources\app_icon.ico
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
WizardStyle=modern
ChangesAssociations=yes
Compression=lzma2
SolidCompression=yes

[Files]
; The whole runner directory, recursively, which is how `lumit-ofx-broker.exe`,
; `lumit-aplug-broker.exe` and `lumit-media-broker.exe` ship without rules of
; their own: the Windows CMake
; install step puts them beside the runner (docs/12 §2.3), and everything
; beside the runner is copied here.
;
; The two exports that ask a laptop for its discrete card make the linker write
; lumit.lib and lumit.exp beside the exe. Neither is any use to a user.
Source: "..\..\flutter_ui\build\windows\x64\runner\Release\*"; DestDir: "{app}"; \
  Excludes: "lumit.lib,lumit.exp"; Flags: recursesubdirs ignoreversion
Source: "..\..\assets\brand\lumit-project.ico"; DestDir: "{app}\icons"
Source: "..\..\assets\brand\lumit-preset.ico"; DestDir: "{app}\icons"
Source: "..\..\assets\brand\lumit-theme.ico"; DestDir: "{app}\icons"

[InstallDelete]
; The runner was lumit_flutter.exe up to 0.3.2. An update over one of those
; installs would leave the old name beside the new one without this.
Type: files; Name: "{app}\lumit_flutter.exe"
; The libraries sat beside the exe up to 0.5.0 and live in lib\ now. Windows
; looks beside the exe first, so an old copy left here would be loaded in
; place of the new one.
Type: files; Name: "{app}\*.dll"
; An OFX plugin's own log, which the plugin brokers used to let land here.
Type: files; Name: "{app}\ofxTestLog.txt"

[Icons]
Name: "{group}\Lumit"; Filename: "{app}\{#MyAppExe}"
Name: "{group}\Uninstall Lumit"; Filename: "{uninstallexe}"

[Registry]
; .lum — project documents
Root: HKA; Subkey: "Software\Classes\.lum"; ValueType: string; \
  ValueData: "Lumit.Project"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Lumit.Project"; ValueType: string; \
  ValueData: "Lumit project"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Lumit.Project\DefaultIcon"; ValueType: string; \
  ValueData: "{app}\icons\lumit-project.ico"
Root: HKA; Subkey: "Software\Classes\Lumit.Project\shell\open\command"; ValueType: string; \
  ValueData: """{app}\{#MyAppExe}"" ""%1"""
; lumit: — invite links. A click on one in a browser starts Lumit with the
; link, and its Shared project window opens on it.
Root: HKA; Subkey: "Software\Classes\lumit"; ValueType: string; \
  ValueData: "URL:Lumit invite link"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\lumit"; ValueType: string; \
  ValueName: "URL Protocol"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\lumit\DefaultIcon"; ValueType: string; \
  ValueData: "{app}\{#MyAppExe},0"
Root: HKA; Subkey: "Software\Classes\lumit\shell\open\command"; ValueType: string; \
  ValueData: """{app}\{#MyAppExe}"" ""%1"""
; .lumfx — presets. No open verb: a preset is applied inside a project, not
; opened on its own, so it gets the icon and a name only.
Root: HKA; Subkey: "Software\Classes\.lumfx"; ValueType: string; \
  ValueData: "Lumit.Preset"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Lumit.Preset"; ValueType: string; \
  ValueData: "Lumit preset"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Lumit.Preset\DefaultIcon"; ValueType: string; \
  ValueData: "{app}\icons\lumit-preset.ico"
; .lumtheme — shared colour themes. No open verb either: a theme is
; taken in from Settings → Appearance → Import…, not opened as a document.
Root: HKA; Subkey: "Software\Classes\.lumtheme"; ValueType: string; \
  ValueData: "Lumit.Theme"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Lumit.Theme"; ValueType: string; \
  ValueData: "Lumit theme"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Lumit.Theme\DefaultIcon"; ValueType: string; \
  ValueData: "{app}\icons\lumit-theme.ico"

[Run]
; No `skipifsilent`. `/SILENT` is how Lumit runs this installer on
; itself to apply an update: it quits, the installer replaces it, and
; without this line nothing starts again — a "Restart now" button that does
; not restart, which is exactly the complaint. Interactive installs are
; unchanged: the `postinstall` flag still offers it as the last checkbox.
Filename: "{app}\{#MyAppExe}"; Description: "Launch Lumit"; \
  Flags: nowait postinstall
