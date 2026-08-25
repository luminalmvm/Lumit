; Lumit Windows installer (Inno Setup, K-252). Build with:
;   packaging/windows/build-installer.ps1
; or by hand: flutter build windows --release (from flutter_ui/), then
;   iscc packaging\windows\lumit.iss
;
; Registers the .lum, .lumfx and .lumtheme associations with their document icons
; (assets/brand, K-251) and an open command; Lumit itself reads the document
; path from the command line (projectPathFromArgs in flutter_ui/lib/main.dart).

; Keep in step with flutter_ui/pubspec.yaml `version:` when cutting a release.
; The release workflow overrides this from the tag (iscc /DMyAppVersion=...).
#ifndef MyAppVersion
#define MyAppVersion "0.1.0"
#endif
#define MyAppExe "lumit_flutter.exe"

[Setup]
AppId={{8B6F1C6A-9E4B-4C7D-B1A4-6C1E5D2F7A31}
AppName=Lumit
AppVersion={#MyAppVersion}
AppPublisher=Lumit
AppPublisherURL=https://github.com/luminalmvm/Lumit
; Per user, not per machine (K-297). This is what lets Lumit update itself the
; way Chrome and VS Code do: {localappdata} belongs to the person running it, so
; the application can put a new version down beside the old one and swap them
; over without an administrator and without running this installer again.
; `PrivilegesRequired=lowest` means no UAC prompt to install in the first place.
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
Source: "..\..\flutter_ui\build\windows\x64\runner\Release\*"; DestDir: "{app}"; \
  Flags: recursesubdirs ignoreversion
Source: "..\..\assets\brand\lumit-project.ico"; DestDir: "{app}\icons"
Source: "..\..\assets\brand\lumit-preset.ico"; DestDir: "{app}\icons"
Source: "..\..\assets\brand\lumit-theme.ico"; DestDir: "{app}\icons"

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
; .lumfx — presets. No open verb: a preset is applied inside a project, not
; opened on its own, so it gets the icon and a name only.
Root: HKA; Subkey: "Software\Classes\.lumfx"; ValueType: string; \
  ValueData: "Lumit.Preset"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Lumit.Preset"; ValueType: string; \
  ValueData: "Lumit preset"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Lumit.Preset\DefaultIcon"; ValueType: string; \
  ValueData: "{app}\icons\lumit-preset.ico"
; .lumtheme — shared colour themes (K-298). No open verb either: a theme is
; taken in from Settings → Appearance → Import…, not opened as a document.
Root: HKA; Subkey: "Software\Classes\.lumtheme"; ValueType: string; \
  ValueData: "Lumit.Theme"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Lumit.Theme"; ValueType: string; \
  ValueData: "Lumit theme"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Lumit.Theme\DefaultIcon"; ValueType: string; \
  ValueData: "{app}\icons\lumit-theme.ico"

[Run]
Filename: "{app}\{#MyAppExe}"; Description: "Launch Lumit"; \
  Flags: nowait postinstall skipifsilent
