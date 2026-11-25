; AutoSpaces installer (plan 12.3). Built by tools\build-installer.ps1, which passes /DAppVersion=<Cargo.toml version>.
#ifndef AppVersion
  #error Pass /DAppVersion=x.y.z (use tools\build-installer.ps1).
#endif

#define AppName "AutoSpaces"
#define AppExe "autospaces.exe"
; Same names as in src\main.rs (SINGLE_INSTANCE without the "Local\" prefix, WINDOW_CLASS).
#define AppMutexName "AutoSpaces_SingleInstance"
#define AppWindowClass "AutoSpacesHiddenWindow"
; Same key and value name as src\sysopts.rs (autostart): the checkbox in the app's settings reads this value.
#define RunKey "Software\Microsoft\Windows\CurrentVersion\Run"

[Setup]
AppId={{30AB1B41-D258-4DAA-A410-484D902466A8}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=Přemysl Vavroušek
AppPublisherURL=https://www.premysl-vavrousek.cz
AppSupportURL=https://www.premysl-vavrousek.cz
AppUpdatesURL=https://www.premysl-vavrousek.cz
AppCopyright=Přemysl Vavroušek
VersionInfoVersion={#AppVersion}.0
VersionInfoCompany=Přemysl Vavroušek
VersionInfoProductName={#AppName}
VersionInfoDescription={#AppName} Setup
; Per-user by default (no admin rights); the dialog offers "for all users" (Program Files, UAC).
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
DefaultDirName={autopf}\{#AppName}
DisableProgramGroupPage=yes
DefaultGroupName={#AppName}
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
; Running app is closed by our own code (clean exit, see [Code]); AppMutex is the fallback prompt.
AppMutex={#AppMutexName}
CloseApplications=no
RestartApplications=no
; Language follows the Windows UI language without asking: Czech on Czech Windows, English otherwise.
ShowLanguageDialog=no
LanguageDetectionMethod=uilanguage
SetupIconFile=..\assets\autospaces.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
OutputDir=..\dist
OutputBaseFilename=AutoSpaces-Setup-{#AppVersion}

[Languages]
; English first: Inno falls back to the first language when the UI language matches none.
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "cs"; MessagesFile: "compiler:Languages\Czech.isl"

[CustomMessages]
en.TaskAutostart=Start AutoSpaces when Windows starts
en.TaskGroupStartup=Startup:
en.LaunchApp=Start AutoSpaces now
en.UninstallDataPrompt=Do you also want to delete AutoSpaces settings and learned data of the current user?%n%nSettings: %1%nData: %2%n%nChoose No to keep them (for a later reinstall).
cs.TaskAutostart=Spouštět AutoSpaces po startu Windows
cs.TaskGroupStartup=Spouštění:
cs.LaunchApp=Spustit AutoSpaces po dokončení
cs.UninstallDataPrompt=Chcete smazat také nastavení a naučená data AutoSpaces aktuálního uživatele?%n%nNastavení: %1%nData: %2%n%nPři volbě Ne zůstanou zachována (pro pozdější novou instalaci).

[Tasks]
Name: "autostart"; Description: "{cm:TaskAutostart}"; GroupDescription: "{cm:TaskGroupStartup}"; Flags: unchecked

[Files]
Source: "..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"

[Registry]
; The app's own autostart is HKCU\...\Run\AutoSpaces = "<exe path>" (src\sysopts.rs). We write the very same value.
; HKCU is deliberate even for an all-users install: HKLM Run would not match the app's setting checkbox and the
; user could not turn it off there. In an all-users install the value is created for the installing user only.
Root: HKCU; Subkey: "{#RunKey}"; ValueType: string; ValueName: "{#AppName}"; ValueData: """{app}\{#AppExe}"""; Tasks: autostart; Flags: uninsdeletevalue

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchApp}"; Flags: nowait postinstall skipifsilent runasoriginaluser
; Update from the app (plan 12.15): it runs this installer with /SILENT ... /UPDATE and has exited (or is closed by
; CloseRunningApp below); start it again as the original, non-elevated user. For all users the installer elevates
; itself (the app does not use "runas"), so the original user is known here.
Filename: "{app}\{#AppExe}"; Flags: nowait runasoriginaluser; Check: IsAppUpdate

[Code]
const
  WM_CLOSE = $0010;

{ Started by AutoSpaces' own update (silent, with /UPDATE on the command line). }
function IsAppUpdate: Boolean;
var
  I: Integer;
begin
  Result := False;
  if not WizardSilent then Exit;
  for I := 1 to ParamCount do
    if CompareText(ParamStr(I), '/UPDATE') = 0 then
    begin
      Result := True;
      Exit;
    end;
end;

function IsAppRunning: Boolean;
begin
  Result := CheckForMutexes('{#AppMutexName}');
end;

{ Asks the running AutoSpaces to exit the same way as tray "Exit" (WM_CLOSE: it gives the windows back and levels
  the desktops) and waits up to ~15 s. If it is still running, AppMutex makes Setup / Uninstall ask the user. }
procedure CloseRunningApp;
var
  Wnd, I: Integer;
begin
  if not IsAppRunning then Exit;
  Wnd := FindWindowByClassName('{#AppWindowClass}');
  if Wnd <> 0 then
    PostMessage(Wnd, WM_CLOSE, 0, 0);
  for I := 1 to 150 do
  begin
    if not IsAppRunning then Break;
    Sleep(100);
  end;
  if not IsAppRunning then
    Sleep(300); { let the process finish unloading before files are replaced }
end;

function InitializeSetup: Boolean;
begin
  CloseRunningApp;
  Result := True;
end;

function InitializeUninstall: Boolean;
begin
  CloseRunningApp;
  Result := True;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Cfg, Dat: String;
begin
  if CurUninstallStep = usUninstall then
  begin
    { Autostart set from the app's settings (not by the installer task) must go away too. }
    RegDeleteValue(HKCU, '{#RunKey}', '{#AppName}');
  end
  else if CurUninstallStep = usPostUninstall then
  begin
    Cfg := ExpandConstant('{userappdata}\{#AppName}');
    Dat := ExpandConstant('{localappdata}\{#AppName}');
    if DirExists(Cfg) or DirExists(Dat) then
      { Default answer (also with /SUPPRESSMSGBOXES) is No = keep. }
      if SuppressibleMsgBox(FmtMessage(CustomMessage('UninstallDataPrompt'), [Cfg, Dat]),
           mbConfirmation, MB_YESNO or MB_DEFBUTTON2, IDNO) = IDYES then
      begin
        DelTree(Cfg, True, True, True);
        DelTree(Dat, True, True, True);
      end;
  end;
end;
