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
; is uninstalled by [Code] below — its leftover files and data folder going with it —
; so an update lands as one program, not two, and as a fresh install.
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
; There is no "keep the old data" choice on an update: the previous install's files
; and data folder go with it (see [Code]), so every update lands as a fresh install
; and the user signs in again. A pre-2.0.3 install kept its data under
; %AppData%\ytm-rs; that folder is ytm-rs's and is never touched.

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
  // BitChord's own AppId, matching [Setup] above.
  OurAppId = '{44C00AFB-5305-4CCA-80C5-5F0D03AC0D29}';
  // ytm-rs's AppId, which every BitChord install before 2.0.3 also used: Windows
  // treated the two apps as one program. Ours is a new one; an old install under
  // the shared ID is uninstalled here so it doesn't linger beside the new entry.
  LegacyAppId = '{6E0B7C4A-2D1F-4C8E-9B3A-7F5D1E2A9C40}';

var
  // Where a previous BitChord install lives, from its uninstall key's
  // InstallLocation ('' when there is none): the folder whose leftover files the
  // update wipes before the new ones land. A previous uninstaller only removes
  // what its own installer laid down — files renamed or added since stay — so an
  // update that just runs it would mix old files into the new install.
  PrevDir: String;

// Read a value of the install's uninstall key under `id`. HKCU covers a per-user
// install, HKLM an all-users one, and the 32-bit view ('WOW6432Node\') an install
// a 32-bit build made. Sets `value` and returns True when the key is there.
function UninstallValue(id: String; name: String; var value: String): Boolean;
var
  path: String;
begin
  path := 'Software\Microsoft\Windows\CurrentVersion\Uninstall\' + id + '_is1';
  Result := RegQueryStringValue(HKEY_CURRENT_USER, path, name, value);
  if not Result then
    Result := RegQueryStringValue(HKEY_LOCAL_MACHINE, path, name, value);
  if not Result then
    Result := RegQueryStringValue(HKEY_LOCAL_MACHINE, 'Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\' + id + '_is1', name, value);
end;

// Where this install goes by default: a pre-2.0.3 install under the shared AppId
// may live somewhere custom — an update keeps that place. A ytm-rs that owns the
// shared key is not ours to follow; the standard folder it is.
function DefaultDir(Param: String): String;
var
  name, location: String;
begin
  Result := ExpandConstant('{autopf}\{#AppName}');
  if not UninstallValue(LegacyAppId, 'DisplayName', name) then exit;
  if Pos('ytm-rs', Lowercase(name)) > 0 then exit;
  if UninstallValue(LegacyAppId, 'InstallLocation', location) and (location <> '') then
    Result := location;
end;

// The previous install's folder, read before setup touches anything: our own
// uninstall key is still in place here — setup runs its uninstaller itself later.
function InitializeSetup(): Boolean;
var
  location: String;
begin
  PrevDir := '';
  if UninstallValue(OurAppId, 'InstallLocation', location) and (location <> '') then
    PrevDir := location;
  Result := True;
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

// A path at a drive or share root is never ours to clear: a stray InstallLocation
// naming one would take everything under it along. A real install folder always
// keeps a folder name past the drive ('X:\name') or the share ('\\srv\share\name').
function RootLike(dir: String): Boolean;
var
  i, depth: Integer;
begin
  Result := True;
  if Copy(dir, 2, 1) = ':' then begin
    Result := Length(dir) <= 3;
    exit;
  end;
  if Copy(dir, 1, 2) = '\\' then begin
    // '\\srv\share' carries one separator past the leading pair; deeper is a folder
    depth := 0;
    for i := 3 to Length(dir) do
      if dir[i] = '\' then
        depth := depth + 1;
    Result := depth < 2;
    exit;
  end;
end;

// An install folder's leftovers: every file and folder still there goes, so the
// new files land alone and nothing old rides along. The folder itself stays when
// the new install is about to fill it again; a root or an empty path is skipped.
procedure WipeLeftovers(dir: String);
begin
  while (Length(dir) > 1) and (Copy(dir, Length(dir), 1) = '\') do
    SetLength(dir, Length(dir) - 1);
  if dir = '' then exit;
  if RootLike(dir) then begin
    Log('Not wiping ' + dir + ': it names a drive or share root.');
    exit;
  end;
  if not DirExists(dir) then exit;
  DelTree(dir, True, True, False);
  Log('Removed an install''s leftover files at ' + dir);
end;

// An old BitChord install under the shared AppId: uninstall it, so this install
// doesn't end up beside a second "BitChord" in Settings › Apps. A ytm-rs that owns
// the shared key is recognized by its DisplayName and left strictly alone. Its
// folder is the previous install's: the uninstaller this runs only removes what
// its own installer laid down, so the leftovers go right after it.
procedure RemoveLegacyInstall;
var
  name, unins, location: String;
begin
  if not UninstallValue(LegacyAppId, 'DisplayName', name) then exit;
  if Pos('ytm-rs', Lowercase(name)) > 0 then begin
    Log('The shared AppId is held by ' + name + '; leaving it alone.');
    exit;
  end;
  if not UninstallValue(LegacyAppId, 'QuietUninstallString', unins) then
    if not UninstallValue(LegacyAppId, 'UninstallString', unins) then begin
      Log('No uninstaller for the old install under the shared AppId; skipping.');
      exit;
    end;
  RunUninstaller(unins);
  if UninstallValue(LegacyAppId, 'InstallLocation', location) then
    WipeLeftovers(location);
end;

// The install under our own AppId, which setup uninstalls itself before this
// runs: its folder gets the same sweep, read back at InitializeSetup (the key is
// gone by the time we get here).
procedure WipePreviousInstall;
begin
  WipeLeftovers(PrevDir);
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

// An update lands as a fresh install: the previous version's leftover files go,
// and the data folder goes with it — an old settings file or cache is exactly
// what leaves an updated app misbehaving, and nothing is migrated in its place.
// The user signs in again. The app is closed by the setup's CloseApplications, so
// nothing holds the files open. %AppData%\ytm-rs is never touched: it is ytm-rs's.
procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssInstall then begin
    RemoveLegacyInstall();
    RemoveStaleScheme();
    WipePreviousInstall();
    DelTree(ExpandConstant('{userappdata}\bitchord'), True, True, True);
    Log('Removed the previous BitChord data (%AppData%\bitchord) with the old install.');
  end;
end;
