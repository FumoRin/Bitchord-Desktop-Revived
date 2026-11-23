; Inno Setup script for BitChord.
; Build: cargo build --release, then
;   iscc installer\bitchord.iss
; Output: installer\Output\bitchord-setup-<version>.exe
; The version comes from Cargo.toml by default; the release workflow stamps its own:
;   iscc /DAppVersion="2.0.3" installer\bitchord.iss
; ARM64: cargo build --release --target aarch64-pc-windows-msvc, then
;   iscc /DArm64 installer\bitchord.iss  ->  bitchord-setup-<version>-arm64.exe

#define AppName "BitChord"
#ifndef AppVersion
#define AppVersion "2.0.2"
#endif
#define AppExe "bitchord.exe"
#ifdef Arm64
  #define Arch "arm64"
  #define Suffix "-arm64"
  #define BinDir "..\target\aarch64-pc-windows-msvc\release"
#else
  #define Arch "x64compatible"
  #define Suffix ""
  #define BinDir "..\target\release"
#endif

[Setup]
; BitChord's own ID, new in 2.0.3. Before that, every install rode on ytm-rs's GUID,
; so Windows treated the two apps as one program: installing either "upgraded" the
; other and they shared a single uninstall entry. An old install under the shared ID
; is uninstalled by [Code] below, so an update lands as one program, not two.
AppId={{44C00AFB-5305-4CCA-80C5-5F0D03AC0D29}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=Loserdenji
AppPublisherURL=https://github.com/LoserDenji/Bitchord-Desktop-Revived
AppSupportURL=https://github.com/LoserDenji/Bitchord-Desktop-Revived/issues
; A pre-2.0.3 install under the shared AppId may live somewhere custom: keep it.
DefaultDirName={code:DefaultDir}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
; Per-user install by default (no UAC prompt); the user can pick all-users instead.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed={#Arch}
ArchitecturesInstallIn64BitMode={#Arch}
MinVersion=10.0
OutputBaseFilename=bitchord-setup-{#AppVersion}{#Suffix}
Compression=lzma2/ultra
SolidCompression=yes
WizardStyle=modern
SetupIconFile=..\assets\icon\icon.ico
UninstallDisplayIcon={app}\{#AppExe}
CloseApplications=yes

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
; The escape hatch for a broken update: an old settings file (app.ron and friends)
; can leave the app misbehaving after an update, and only wiping the data folder
; fixes it. Checked here, everything under %AppData%\bitchord goes before the new
; files land — the app starts as a fresh install. A pre-2.0.3 install kept its data
; under %AppData%\ytm-rs; that folder is ytm-rs's and is never touched.
Name: "cleandata"; Description: "Remove the existing BitChord data before installing (%AppData%\bitchord). Use this if an update left the app misbehaving: it deletes the old settings and caches, so BitChord starts as a fresh install."; GroupDescription: "Fix an update:"; Flags: unchecked

[Files]
; The exe statically links the WebView2 loader, so there's no DLL to ship.
Source: "{#BinDir}\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{group}\Uninstall {#AppName}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Registry]
; bitchord:// links (jam and party invites). HKA is HKCU for per-user installs, HKLM
; for all-users. The installed app also re-registers itself under HKCU on launch.
; Until 2.0.3 this also registered ytm-rs's scheme, so BitChord stole ytm-rs's jam
; invites; the stale registration is cleaned up in [Code].
Root: HKA; Subkey: "Software\Classes\bitchord"; ValueType: string; ValueName: ""; ValueData: "URL:bitchord Protocol"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\bitchord"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\bitchord\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"",0"
Root: HKA; Subkey: "Software\Classes\bitchord\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""

[UninstallDelete]
; Installers downloaded by the in-app updater.
Type: filesandordirs; Name: "{userappdata}\bitchord\data\updates"
; The app's own Start menu shortcut on an all-users install, for a custom app icon.
Type: files; Name: "{userprograms}\{#AppName}\{#AppName}.lnk"
Type: dirifempty; Name: "{userprograms}\{#AppName}"

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[Code]
const
  // ytm-rs's AppId, which every BitChord install before 2.0.3 also used: Windows
  // treated the two apps as one program. Ours is a new one; an old install under
  // the shared ID is uninstalled here so it doesn't linger beside the new entry.
  LegacyAppId = '{6E0B7C4A-2D1F-4C8E-9B3A-7F5D1E2A9C40}';

// Read a value of the old install's uninstall key. `view` covers the 32-bit view
// ('WOW6432Node\'), for installs a 32-bit build made. Sets `value` and returns True
// when the key is there.
function LegacyValue(view: String; name: String; var value: String): Boolean;
var
  path: String;
begin
  path := 'Software\' + view + 'Microsoft\Windows\CurrentVersion\Uninstall\' + LegacyAppId + '_is1';
  Result := RegQueryStringValue(HKEY_CURRENT_USER, path, name, value);
  if not Result then
    Result := RegQueryStringValue(HKEY_LOCAL_MACHINE, path, name, value);
end;

// Where this install goes by default: a pre-2.0.3 install under the shared AppId
// may live somewhere custom — an update keeps that place. A ytm-rs that owns the
// shared key is not ours to follow; the standard folder it is.
function DefaultDir(Param: String): String;
var
  name, location: String;
begin
  Result := ExpandConstant('{autopf}\{#AppName}');
  if not LegacyValue('', 'DisplayName', name) then exit;
  if Pos('ytm-rs', Lowercase(name)) > 0 then exit;
  if LegacyValue('', 'InstallLocation', location) and (location <> '') then
    Result := location;
end;

// Split "…unins000.exe" /flags into the exe and its flags, then run it with ours.
procedure RunUninstaller(unins: String);
var
  p: Integer;
  exe, params: String;
  code: Integer;
  i: Integer;
begin
  unins := Trim(unins);
  if Pos('"', unins) = 1 then begin
    p := Pos('" ', unins);
    if p = 0 then begin
      // Just "…unins000.exe", no flags after it.
      exe := Copy(unins, 2, Length(unins) - 2);
      params := '';
    end else begin
      exe := Copy(unins, 2, p - 2);
      params := Copy(unins, p + 2, Length(unins) - p - 1);
    end;
  end else begin
    p := Pos(' ', unins);
    if p = 0 then begin
      exe := unins;
      params := '';
    end else begin
      exe := Copy(unins, 1, p - 1);
      params := Copy(unins, p + 1, Length(unins) - p);
    end;
  end;
  Log('Running the old uninstaller: ' + exe + ' ' + params);
  Exec(exe, params + ' /VERYSILENT /SUPPRESSMSGBOXES /NORESTART', '', SW_SHOW, ewWaitUntilTerminated, code);
  // The uninstaller runs from a temp copy of itself and returns before that copy
  // is done, so wait for the old files to be gone before the new ones land.
  for i := 0 to 59 do begin
    if not FileExists(exe) then exit;
    Sleep(250);
  end;
  Log('The old uninstaller is taking its time; carrying on.');
end;

// An old BitChord install under the shared AppId: uninstall it, so this install
// doesn't end up beside a second "BitChord" in Settings › Apps. A ytm-rs that owns
// the shared key is recognized by its DisplayName and left strictly alone.
procedure RemoveLegacyInstall;
var
  name, unins: String;
begin
  if not LegacyValue('', 'DisplayName', name) then exit;
  if Pos('ytm-rs', Lowercase(name)) > 0 then begin
    Log('The shared AppId is held by ' + name + '; leaving it alone.');
    exit;
  end;
  if not LegacyValue('', 'QuietUninstallString', unins) then
    if not LegacyValue('', 'UninstallString', unins) then begin
      Log('No uninstaller for the old install under the shared AppId; skipping.');
      exit;
    end;
  RunUninstaller(unins);
end;

// The ytm-rs:// class registration a pre-2.0.3 install wrote: give the scheme back
// to ytm-rs, but only where it opens a BitChord exe. A registration pointing at
// ytm-rs's own exe is ytm-rs's, and stays. HKLM needs the elevated half of an
// all-users install; where it isn't writable, the app's own launch does the HKCU
// half (see deeplink.rs).
procedure RemoveStaleSchemeAt(root: Integer);
var
  cmd: String;
begin
  if RegQueryStringValue(root, 'Software\Classes\ytm-rs\shell\open\command', '', cmd) then
    if Pos('bitchord.exe', Lowercase(cmd)) > 0 then begin
      RegDeleteKeyIncludingSubkeys(root, 'Software\Classes\ytm-rs');
      Log('Removed this app''s ytm-rs:// registration.');
    end;
end;

procedure RemoveStaleScheme;
begin
  RemoveStaleSchemeAt(HKEY_CURRENT_USER);
  RemoveStaleSchemeAt(HKEY_LOCAL_MACHINE);
end;

// The "Remove the existing BitChord data" task: the data folder goes just before
// the new files are written, so the install that follows is a clean one. The app
// is closed by the setup's CloseApplications, so nothing holds the files open.
procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssInstall then begin
    RemoveLegacyInstall();
    RemoveStaleScheme();
    if IsTaskSelected('cleandata') then begin
      DelTree(ExpandConstant('{userappdata}\bitchord'), True, True, True);
      Log('Removed existing BitChord data (%AppData%\bitchord) at the user''s request.');
    end;
  end;
end;
