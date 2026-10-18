; MetalAMP installer (Inno Setup 6).
; Built by tools\make_release.py, which passes the version and the icon path:
;   ISCC /DMyAppVersion=0.1.0 /DVersionInfoVersion=0.1.0.0 /DAppIcon=..\target\app.ico tools\setup.iss

#define MyAppName "MetalAMP"
#define MyAppFileStem "MetalAMP"
#define MyAppExeName "metalamp.exe"
#define MyAppPublisher "Fan4_Metal"
#ifndef MyAppVersion
  #define MyAppVersion "0.0.0"
#endif
#ifndef VersionInfoVersion
  #define VersionInfoVersion "0.0.0.0"
#endif
#ifndef AppIcon
  #define AppIcon "..\target\app.ico"
#endif

[Setup]
; A new GUID for every app (the same one in tools\make_release.py).
AppId={{9F2FF736-EA36-48CC-9BE4-FDE257B5AC53}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
VersionInfoVersion={#VersionInfoVersion}
AppCopyright=Copyright (C) 2026 {#MyAppPublisher}
AppPublisher={#MyAppPublisher}
; Per-user installation without administrator rights:
; {autopf} points to %LOCALAPPDATA%\Programs.
PrivilegesRequired=lowest
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
OutputDir=..\dist
OutputBaseFilename={#MyAppFileStem}_{#MyAppVersion}_Setup
SetupIconFile={#AppIcon}
UninstallDisplayIcon={app}\{#MyAppExeName}
LicenseFile=..\LICENSE
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
; Windows 10 1607 or later: Media Foundation's FLAC decoder came with it.
MinVersion=10.0.14393
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; A running copy is closed (Restart Manager) before its exe is replaced.
CloseApplications=yes
; File associations change: Explorer is told to refresh its icons.
ChangesAssociations=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"

[CustomMessages]
english.AssocGroup=File types:
russian.AssocGroup=Типы файлов:
english.AssocTask=Register MetalAMP for audio files and playlists (MP3, FLAC, M4A, AAC, WAV, WMA, M3U)
russian.AssocTask=Зарегистрировать MetalAMP для аудиофайлов и плейлистов (MP3, FLAC, M4A, AAC, WAV, WMA, M3U)
english.AssocStatus=Registering file types...
russian.AssocStatus=Регистрация типов файлов...
english.ChooseDefault=Choose MetalAMP as the default player (opens Windows Settings)
russian.ChooseDefault=Выбрать MetalAMP проигрывателем по умолчанию (откроются параметры Windows)
english.DeleteSettings=Delete the settings, the library index and the covers of MetalAMP as well (%1)?
russian.DeleteSettings=Удалить и настройки, индекс библиотеки и обложки MetalAMP (%1)?

[Tasks]
Name: "assoc"; Description: "{cm:AssocTask}"; GroupDescription: "{cm:AssocGroup}"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\licenses\THIRD_PARTY_NOTICES.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.ru.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
; The file types are registered by the program itself (src\assoc.rs, under
; HKCU): a ProgID with an icon for each format, "Open with" entries and
; Default apps capabilities. Windows lets only the user choose the default
; program, so the last page offers to open Settings for that.
Filename: "{app}\{#MyAppExeName}"; Parameters: "--register"; StatusMsg: "{cm:AssocStatus}"; Flags: runhidden waituntilterminated; Tasks: assoc
Filename: "ms-settings:defaultapps?registeredAppUser={#MyAppName}"; Description: "{cm:ChooseDefault}"; Flags: shellexec nowait postinstall skipifsilent unchecked; Tasks: assoc
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#MyAppName}}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Removes everything --register wrote; nothing happens if it was not run.
Filename: "{app}\{#MyAppExeName}"; Parameters: "--unregister"; RunOnceId: "UnregisterFileTypes"; Flags: runhidden waituntilterminated

[Code]
// A copy running from the installed folder is closed first, as by its
// window's cross (its settings saved): else its exe stays behind, and it
// would write its settings again after they were deleted. Copies elsewhere
// (a portable one) are left alone.
function InitializeUninstall(): Boolean;
var
  Code: Integer;
begin
  Exec('powershell.exe',
    '-NoProfile -NonInteractive -Command "$p = Get-Process metalamp -ErrorAction SilentlyContinue | ' +
    'Where-Object { $_.Path -eq ''' + ExpandConstant('{app}\{#MyAppExeName}') + ''' }; ' +
    '$p | ForEach-Object { [void]$_.CloseMainWindow() }; ' +
    '$p | Wait-Process -Timeout 5 -ErrorAction SilentlyContinue"',
    '', SW_HIDE, ewWaitUntilTerminated, Code);
  Result := True;
end;

// The settings, the library index and the covers are left unless the user
// wants them gone (qrec's way); a silent uninstallation leaves them.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Settings: String;
begin
  if CurUninstallStep = usPostUninstall then
  begin
    Settings := ExpandConstant('{userappdata}\{#MyAppName}');
    if DirExists(Settings) and not UninstallSilent then
      if MsgBox(FmtMessage(CustomMessage('DeleteSettings'), [Settings]), mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
        DelTree(Settings, True, True, True);
  end;
end;
