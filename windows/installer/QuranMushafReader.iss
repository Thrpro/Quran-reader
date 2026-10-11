#define AppName "Quran Mushaf Reader"
#ifndef AppVersion
  #define AppVersion "0.5.0"
#endif
#ifndef NativeHost
  #define NativeHost "rust"
#endif
#define Publisher "Thrpro"
#define RepositoryUrl "https://github.com/Thrpro/Quran-reader"
#define ProjectRoot "..\QuranMushaf.Windows"
#ifndef PublishDir
  #if NativeHost == "legacy"
    #define PublishDir ProjectRoot + "\bin\Release\net10.0-windows10.0.19041.0\win-x64\publish"
  #else
    #define PublishDir "..\..\outputs\rust-windows\publish"
  #endif
#endif
#ifndef OutputDir
  #define OutputDir "..\..\outputs\releases\v" + AppVersion
#endif
#if NativeHost == "legacy"
  #define InstallerNotice "INSTALLER-NOTICE-LEGACY.txt"
#else
  #define InstallerNotice "INSTALLER-NOTICE.txt"
#endif

[Setup]
#if !FileExists(PublishDir + "\WebApp\experimental\digital-khatt\shaped\glyphs.json")
  #error Missing required Digital Khatt glyph data. Rebuild and publish before packaging.
#endif
#if !FileExists(PublishDir + "\WebApp\experimental\digital-khatt\shaped\p604.json")
  #error Missing Digital Khatt page data. Rebuild and publish before packaging.
#endif
AppId={{B143ABF5-90F2-4A70-90E9-35FE5E9D6E72}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher={#Publisher}
AppPublisherURL={#RepositoryUrl}
AppSupportURL={#RepositoryUrl}/issues
AppUpdatesURL={#RepositoryUrl}/releases/latest
DefaultDirName={localappdata}\Programs\QuranMushafReader
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
OutputDir={#OutputDir}
OutputBaseFilename=QuranMushafReader-Setup-x64-{#AppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.19041
PrivilegesRequired=lowest
UsedUserAreasWarning=no
UsePreviousAppDir=yes
CloseApplications=force
RestartApplications=no
SetupLogging=yes
SetupIconFile={#PublishDir}\Assets\app.ico
UninstallDisplayIcon={app}\QuranMushaf.Windows.exe
UninstallDisplayName={#AppName}
InfoBeforeFile={#InstallerNotice}

[Files]
Source: "{#PublishDir}\*"; DestDir: "{app}"; Excludes: "*.pdb,Install-QuranMushafReader.cmd,Uninstall-QuranMushafReader.cmd,QuranMushaf.Windows.exe.WebView2\*,WebApp\experimental\digital-khatt\shaped\*"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#PublishDir}\WebApp\experimental\digital-khatt\shaped\*"; DestDir: "{app}\WebApp\experimental\digital-khatt\shaped"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "..\..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\THIRD-PARTY-NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#InstallerNotice}"; DestDir: "{app}"; DestName: "SOURCE-CODE.txt"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\QuranMushaf.Windows.exe"
Name: "{group}\Source code and releases"; Filename: "{#RepositoryUrl}"
Name: "{group}\Uninstall {#AppName}"; Filename: "{uninstallexe}"
Name: "{userdesktop}\{#AppName}"; Filename: "{app}\QuranMushaf.Windows.exe"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked

[InstallDelete]
; These are app-owned runtime files. Clear obsolete native files on upgrades;
; user settings, tafsir and recitations live in a separate AppData directory.
Type: files; Name: "{app}\*.dll"
Type: files; Name: "{app}\WebApp\assets\index-*.js"
Type: files; Name: "{app}\WebApp\assets\index-*.css"
Type: files; Name: "{userprograms}\Quran Mushaf Reader - Fresh Build.lnk"
Type: files; Name: "{userdesktop}\Quran Mushaf Reader - Fresh Build.lnk"
Type: files; Name: "{app}\Install-QuranMushafReader.cmd"
Type: files; Name: "{app}\Uninstall-QuranMushafReader.cmd"
Type: filesandordirs; Name: "{app}\QuranMushaf.Windows.exe.WebView2"

#if NativeHost == "rust"
; Known app-owned files from the previous .NET/WinUI release. Keep the shared
; QuranMushafReader\WebView2 user profile and all WebApp resources intact.
Type: files; Name: "{app}\createdump.exe"
Type: files; Name: "{app}\RestartAgent.exe"
Type: files; Name: "{app}\App.xbf"
Type: files; Name: "{app}\MainWindow.xbf"
Type: files; Name: "{app}\Microsoft.UI.pri"
Type: files; Name: "{app}\Microsoft.UI.Xaml.Controls.pri"
Type: files; Name: "{app}\Microsoft.WindowsAppRuntime.pri"
Type: files; Name: "{app}\QuranMushaf.Windows.deps.json"
Type: files; Name: "{app}\QuranMushaf.Windows.runtimeconfig.json"
Type: files; Name: "{app}\QuranMushaf.Windows.pdb"
Type: files; Name: "{app}\WindowsAppRuntime.png"
Type: files; Name: "{app}\Assets\app-256.png"
Type: files; Name: "{app}\Assets\SplashScreen.png"
Type: files; Name: "{app}\Assets\Square150x150Logo.png"
Type: files; Name: "{app}\Assets\Square44x44Logo.png"
Type: files; Name: "{app}\Assets\StoreLogo.png"
Type: files; Name: "{app}\Assets\Wide310x150Logo.png"
Type: files; Name: "{app}\workloads.365.json"
Type: files; Name: "{app}\workloads.j32.json"
Type: files; Name: "{app}\workloads.json"
Type: files; Name: "{app}\workloads.lnl.json"
Type: files; Name: "{app}\workloads.qnn.json"
Type: files; Name: "{app}\workloads.stx.json"
Type: files; Name: "{app}\Microsoft.Foundation.winmd"
Type: files; Name: "{app}\Microsoft.Graphics.Imaging.winmd"
Type: files; Name: "{app}\Microsoft.Graphics.ImagingInternal.ImageObjectRemover.winmd"
Type: files; Name: "{app}\Microsoft.Graphics.ImagingInternal.winmd"
Type: files; Name: "{app}\Microsoft.Graphics.Internal.Imaging.winmd"
Type: files; Name: "{app}\Microsoft.Graphics.winmd"
Type: files; Name: "{app}\Microsoft.Security.Authentication.OAuth.winmd"
Type: files; Name: "{app}\Microsoft.UI.Text.winmd"
Type: files; Name: "{app}\Microsoft.UI.winmd"
Type: files; Name: "{app}\Microsoft.UI.Xaml.winmd"
Type: files; Name: "{app}\Microsoft.Windows.AI.ContentModerationInternal.winmd"
Type: files; Name: "{app}\Microsoft.Windows.AI.ContentSafety.winmd"
Type: files; Name: "{app}\Microsoft.Windows.AI.Foundation.winmd"
Type: files; Name: "{app}\Microsoft.Windows.AI.FoundationInternal.winmd"
Type: files; Name: "{app}\Microsoft.Windows.AI.GenerativeInternal.winmd"
Type: files; Name: "{app}\Microsoft.Windows.AI.Imaging.winmd"
Type: files; Name: "{app}\Microsoft.Windows.AI.MachineLearning.winmd"
Type: files; Name: "{app}\Microsoft.Windows.AI.Text.winmd"
Type: files; Name: "{app}\Microsoft.Windows.AI.winmd"
Type: files; Name: "{app}\Microsoft.Windows.ApplicationModel.Background.UniversalBGTask.winmd"
Type: files; Name: "{app}\Microsoft.Windows.ApplicationModel.Background.winmd"
Type: files; Name: "{app}\Microsoft.Windows.ApplicationModel.DynamicDependency.winmd"
Type: files; Name: "{app}\Microsoft.Windows.ApplicationModel.Resources.winmd"
Type: files; Name: "{app}\Microsoft.Windows.ApplicationModel.WindowsAppRuntime.winmd"
Type: files; Name: "{app}\Microsoft.Windows.AppLifecycle.winmd"
Type: files; Name: "{app}\Microsoft.Windows.AppNotifications.Builder.winmd"
Type: files; Name: "{app}\Microsoft.Windows.AppNotifications.winmd"
Type: files; Name: "{app}\Microsoft.Windows.BadgeNotifications.winmd"
Type: files; Name: "{app}\Microsoft.Windows.Foundation.winmd"
Type: files; Name: "{app}\Microsoft.Windows.Globalization.winmd"
Type: files; Name: "{app}\Microsoft.Windows.Internal.Vision.winmd"
Type: files; Name: "{app}\Microsoft.Windows.Management.Deployment.winmd"
Type: files; Name: "{app}\Microsoft.Windows.Media.Capture.winmd"
Type: files; Name: "{app}\Microsoft.Windows.Private.Workloads.SessionManager.winmd"
Type: files; Name: "{app}\Microsoft.Windows.PrivateCommon.winmd"
Type: files; Name: "{app}\Microsoft.Windows.PushNotifications.winmd"
Type: files; Name: "{app}\Microsoft.Windows.Security.AccessControl.winmd"
Type: files; Name: "{app}\Microsoft.Windows.SemanticSearch.winmd"
Type: files; Name: "{app}\Microsoft.Windows.Storage.Pickers.winmd"
Type: files; Name: "{app}\Microsoft.Windows.Storage.winmd"
Type: files; Name: "{app}\Microsoft.Windows.System.Power.winmd"
Type: files; Name: "{app}\Microsoft.Windows.System.winmd"
Type: files; Name: "{app}\Microsoft.Windows.Vision.winmd"
Type: files; Name: "{app}\Microsoft.Windows.VisionInternal.winmd"
Type: files; Name: "{app}\Microsoft.Windows.Widgets.winmd"
Type: files; Name: "{app}\Microsoft.Windows.Workloads.winmd"
Type: filesandordirs; Name: "{app}\af-ZA"
Type: filesandordirs; Name: "{app}\am-ET"
Type: filesandordirs; Name: "{app}\ar-SA"
Type: filesandordirs; Name: "{app}\as-IN"
Type: filesandordirs; Name: "{app}\az-Latn-AZ"
Type: filesandordirs; Name: "{app}\bg-BG"
Type: filesandordirs; Name: "{app}\bn-IN"
Type: filesandordirs; Name: "{app}\bs-Latn-BA"
Type: filesandordirs; Name: "{app}\ca-ES"
Type: filesandordirs; Name: "{app}\ca-Es-VALENCIA"
Type: filesandordirs; Name: "{app}\cs-CZ"
Type: filesandordirs; Name: "{app}\cy-GB"
Type: filesandordirs; Name: "{app}\da-DK"
Type: filesandordirs; Name: "{app}\de-DE"
Type: filesandordirs; Name: "{app}\el-GR"
Type: filesandordirs; Name: "{app}\en-GB"
Type: filesandordirs; Name: "{app}\en-us"
Type: filesandordirs; Name: "{app}\es-ES"
Type: filesandordirs; Name: "{app}\es-MX"
Type: filesandordirs; Name: "{app}\et-EE"
Type: filesandordirs; Name: "{app}\eu-ES"
Type: filesandordirs; Name: "{app}\fa-IR"
Type: filesandordirs; Name: "{app}\fi-FI"
Type: filesandordirs; Name: "{app}\fil-PH"
Type: filesandordirs; Name: "{app}\fr-CA"
Type: filesandordirs; Name: "{app}\fr-FR"
Type: filesandordirs; Name: "{app}\ga-IE"
Type: filesandordirs; Name: "{app}\gd-gb"
Type: filesandordirs; Name: "{app}\gl-ES"
Type: filesandordirs; Name: "{app}\gu-IN"
Type: filesandordirs; Name: "{app}\he-IL"
Type: filesandordirs; Name: "{app}\hi-IN"
Type: filesandordirs; Name: "{app}\hr-HR"
Type: filesandordirs; Name: "{app}\hu-HU"
Type: filesandordirs; Name: "{app}\hy-AM"
Type: filesandordirs; Name: "{app}\id-ID"
Type: filesandordirs; Name: "{app}\is-IS"
Type: filesandordirs; Name: "{app}\it-IT"
Type: filesandordirs; Name: "{app}\ja-JP"
Type: filesandordirs; Name: "{app}\ka-GE"
Type: filesandordirs; Name: "{app}\kk-KZ"
Type: filesandordirs; Name: "{app}\km-KH"
Type: filesandordirs; Name: "{app}\kn-IN"
Type: filesandordirs; Name: "{app}\ko-KR"
Type: filesandordirs; Name: "{app}\kok-IN"
Type: filesandordirs; Name: "{app}\lb-LU"
Type: filesandordirs; Name: "{app}\lo-LA"
Type: filesandordirs; Name: "{app}\lt-LT"
Type: filesandordirs; Name: "{app}\lv-LV"
Type: filesandordirs; Name: "{app}\mi-NZ"
Type: filesandordirs; Name: "{app}\Microsoft.UI.Xaml"
Type: filesandordirs; Name: "{app}\mk-MK"
Type: filesandordirs; Name: "{app}\ml-IN"
Type: filesandordirs; Name: "{app}\mr-IN"
Type: filesandordirs; Name: "{app}\ms-MY"
Type: filesandordirs; Name: "{app}\mt-MT"
Type: filesandordirs; Name: "{app}\nb-NO"
Type: filesandordirs; Name: "{app}\ne-NP"
Type: filesandordirs; Name: "{app}\nl-NL"
Type: filesandordirs; Name: "{app}\nn-NO"
Type: filesandordirs; Name: "{app}\NpuDetect"
Type: filesandordirs; Name: "{app}\or-IN"
Type: filesandordirs; Name: "{app}\pa-IN"
Type: filesandordirs; Name: "{app}\pl-PL"
Type: filesandordirs; Name: "{app}\pt-BR"
Type: filesandordirs; Name: "{app}\pt-PT"
Type: filesandordirs; Name: "{app}\quz-PE"
Type: filesandordirs; Name: "{app}\ro-RO"
Type: filesandordirs; Name: "{app}\ru-RU"
Type: filesandordirs; Name: "{app}\sk-SK"
Type: filesandordirs; Name: "{app}\sl-SI"
Type: filesandordirs; Name: "{app}\sq-AL"
Type: filesandordirs; Name: "{app}\sr-Cyrl-BA"
Type: filesandordirs; Name: "{app}\sr-Cyrl-RS"
Type: filesandordirs; Name: "{app}\sr-Latn-RS"
Type: filesandordirs; Name: "{app}\sv-SE"
Type: filesandordirs; Name: "{app}\ta-IN"
Type: filesandordirs; Name: "{app}\te-IN"
Type: filesandordirs; Name: "{app}\th-TH"
Type: filesandordirs; Name: "{app}\tr-TR"
Type: filesandordirs; Name: "{app}\tt-RU"
Type: filesandordirs; Name: "{app}\ug-CN"
Type: filesandordirs; Name: "{app}\uk-UA"
Type: filesandordirs; Name: "{app}\ur-PK"
Type: filesandordirs; Name: "{app}\uz-Latn-UZ"
Type: filesandordirs; Name: "{app}\vi-VN"
Type: filesandordirs; Name: "{app}\zh-CN"
Type: filesandordirs; Name: "{app}\zh-TW"
#endif

[UninstallDelete]
Type: filesandordirs; Name: "{app}\QuranMushaf.Windows.exe.WebView2"

[Run]
Filename: "{app}\QuranMushaf.Windows.exe"; Description: "Launch {#AppName}"; Flags: nowait postinstall skipifsilent

[Code]
const
  WebView2BootstrapperUrl = 'https://go.microsoft.com/fwlink/p/?LinkId=2124703';

function RunPowerShellCheck(Command: String): Boolean;
var
  ResultCode: Integer;
begin
  Result := Exec(
    ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'),
    '-NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "' + Command + '"',
    '',
    SW_HIDE,
    ewWaitUntilTerminated,
    ResultCode) and (ResultCode = 0);
end;

function QuotePowerShellLiteral(Value: String): String;
begin
  StringChangeEx(Value, '''', '''''', True);
  Result := '''' + Value + '''';
end;

function IsWebView2RuntimeInstalled(): Boolean;
begin
  Result := RunPowerShellCheck(
    '$paths = @(''HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'', ' +
    '''HKCU:\Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}''); ' +
    'foreach ($path in $paths) { $version = (Get-ItemProperty -LiteralPath $path -Name pv ' +
    '-ErrorAction SilentlyContinue).pv; if ($version -and $version -ne ''0.0.0.0'') { exit 0 } }; exit 1');
end;

function HasValidMicrosoftSignature(FileName: String): Boolean;
begin
  Result := RunPowerShellCheck(
    '$s = Get-AuthenticodeSignature -LiteralPath ' + QuotePowerShellLiteral(FileName) + '; ' +
    'if ($s.Status -eq ''Valid'' -and ' +
    '$s.SignerCertificate.Subject -like ''*Microsoft Corporation*'') ' +
    '{ exit 0 } else { exit 1 }');
end;

function InstallMicrosoftPrerequisite(
  Url: String;
  FileName: String;
  Parameters: String;
  DisplayName: String;
  Elevate: Boolean;
  var NeedsRestart: Boolean): String;
var
  DownloadedFile: String;
  ResultCode: Integer;
  Executed: Boolean;
begin
  Result := '';
  WizardForm.StatusLabel.Caption := 'Downloading ' + DisplayName + ' from Microsoft...';

  try
    DownloadTemporaryFile(Url, FileName, '', nil);
  except
    Result := 'Setup could not download ' + DisplayName + '. ' + GetExceptionMessage;
    Exit;
  end;

  DownloadedFile := ExpandConstant('{tmp}\' + FileName);
  if not HasValidMicrosoftSignature(DownloadedFile) then
  begin
    Result := DisplayName + ' did not have a valid Microsoft digital signature. Setup stopped without running it.';
    Exit;
  end;

  WizardForm.StatusLabel.Caption := 'Installing ' + DisplayName + '...';
  if Elevate then
    Executed := ShellExec(
      'runas', DownloadedFile, Parameters, '', SW_SHOWNORMAL,
      ewWaitUntilTerminated, ResultCode)
  else
    Executed := Exec(
      DownloadedFile, Parameters, '', SW_HIDE,
      ewWaitUntilTerminated, ResultCode);

  if (not Executed) or ((ResultCode <> 0) and (ResultCode <> 3010)) then
  begin
    Result := DisplayName + ' installation failed with exit code ' + IntToStr(ResultCode) + '.';
    Exit;
  end;

  if ResultCode = 3010 then
    NeedsRestart := True;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';

  { The Rust host needs no .NET or Windows App Runtime. The legacy build
    includes those runtimes in its self-contained payload. }
  if not IsWebView2RuntimeInstalled() then
  begin
    Result := InstallMicrosoftPrerequisite(
      WebView2BootstrapperUrl,
      'MicrosoftEdgeWebView2Setup.exe',
      '/silent /install',
      'Microsoft Edge WebView2 Runtime',
      False,
      NeedsRestart);
    if Result <> '' then
      Exit;
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  UserDataDir: String;
begin
  if CurUninstallStep <> usPostUninstall then
    Exit;

  UserDataDir := ExpandConstant('{localappdata}\QuranMushafReader');
  if DirExists(UserDataDir) and
     (MsgBox(
       'Remove reading progress, preferences, cached data, and downloaded audio too?',
       mbConfirmation,
       MB_YESNO or MB_DEFBUTTON2) = IDYES) then
    DelTree(UserDataDir, True, True, True);
end;






