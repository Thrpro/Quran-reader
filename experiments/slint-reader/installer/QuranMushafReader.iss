; Installer for the native Quran Mushaf Reader (Slint). Build with Inno Setup 6:
;   ISCC.exe /DAppVersion=0.16.0 /DAppDir=<app folder> /DOutputDir=<output folder> QuranMushafReader.iss
; The app folder is the release folder: QuranMushaf.SlintReader.exe, Resources,
; licenses, the runtime DLLs, resource-manifest.json and the verifier script.

#define AppName "Quran Mushaf Reader"
#ifndef AppVersion
  #error Pass /DAppVersion=x.y.z
#endif
#ifndef AppDir
  #error Pass /DAppDir=<release app folder>
#endif
#ifndef OutputDir
  #define OutputDir "."
#endif
#define AppExe "QuranMushaf.SlintReader.exe"
#define Publisher "Thrpro"
#define RepositoryUrl "https://github.com/Thrpro/Quran-reader"

#if !FileExists(AppDir + "\" + AppExe)
  #error The app folder has no QuranMushaf.SlintReader.exe
#endif
#if !FileExists(AppDir + "\resource-manifest.json")
  #error The app folder has no Quran resource manifest
#endif
#if !FileExists(AppDir + "\vcruntime140.dll")
  #error The app folder is missing the Visual C++ runtime files
#endif

[Setup]
; Its own identity, separate from the earlier WebView2 reader.
AppId={{6F0B9C1E-4D2A-4E8B-9A57-3C1D2E7F8A90}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#Publisher}
AppPublisherURL={#RepositoryUrl}
AppSupportURL={#RepositoryUrl}/issues
AppUpdatesURL={#RepositoryUrl}/releases
VersionInfoVersion={#AppVersion}
VersionInfoProductName={#AppName}
VersionInfoDescription={#AppName} setup
; Per-user install: no administrator prompt, and the app can keep its settings
; in its own folder.
PrivilegesRequired=lowest
DefaultDirName={localappdata}\Programs\Quran Mushaf Reader
DisableProgramGroupPage=yes
DisableDirPage=auto
UsePreviousAppDir=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#OutputDir}
OutputBaseFilename=QuranMushafReader-Setup-x64-{#AppVersion}
SetupIconFile=..\assets\app.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
CloseApplications=yes
RestartApplications=no
SetupLogging=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "arabic"; MessagesFile: "compiler:Languages\Arabic.isl"

[CustomMessages]
english.DesktopIconTask=Create a &desktop shortcut
arabic.DesktopIconTask=إنشاء اختصار على سطح المكتب
english.LaunchApp=Open {#AppName}
arabic.LaunchApp=فتح قارئ المصحف

[Tasks]
Name: "desktopicon"; Description: "{cm:DesktopIconTask}"; Flags: unchecked

[Files]
Source: "{#AppDir}\*"; DestDir: "{app}"; Excludes: "SlintProfile\*,SlintProfile"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{userprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"; WorkingDir: "{app}"
Name: "{userdesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; WorkingDir: "{app}"; Description: "{cm:LaunchApp}"; Flags: nowait postinstall skipifsilent

; Reading position, bookmarks and settings (SlintProfile) are kept on uninstall,
; so reinstalling or upgrading never loses them.
