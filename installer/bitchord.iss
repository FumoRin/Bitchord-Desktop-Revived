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
  GenericCredential = 1;
  CredentialNotFound = 1168;
  CredentialServiceSuffix = '.bitchord';
  MaxCredentialTargetLength = 32767;

var
  // Where a previous BitChord install lives, from its uninstall key's
  // InstallLocation ('' when there is none): the folder whose leftover files the
  // update wipes before the new ones land. A previous uninstaller only removes
  // what its own installer laid down — files renamed or added since stay — so an
  // update that just runs it would mix old files into the new install.
  PrevDir: String;

// Keep the registry root and view with the first matching value so related
// values can be read from that same registration.
function FindUninstallValue(id, name: String; var root: Integer; var path, value: String): Boolean;
begin
  path := 'Software\Microsoft\Windows\CurrentVersion\Uninstall\' + id + '_is1';
  root := HKEY_CURRENT_USER;
  Result := RegQueryStringValue(root, path, name, value);
  if Result then exit;
  root := HKEY_LOCAL_MACHINE;
  Result := RegQueryStringValue(root, path, name, value);
  if Result or not IsWin64 then exit;
  root := HKEY_LOCAL_MACHINE_32;
  Result := RegQueryStringValue(root, path, name, value);
end;

function FindLegacyRegistration(var root: Integer; var path: String): Boolean;
var
  name: String;
begin
  Result := False;
  if not FindUninstallValue(LegacyAppId, 'DisplayName', root, path, name) then exit;
  name := Lowercase(Trim(name));
  // Inno's default display name is AppName or "AppName version AppVersion".
  Result := (name = 'bitchord') or (Pos('bitchord version ', name) = 1);
  if not Result then
    Log('The shared AppId has a different display name; leaving it alone.');
end;

// Accept absolute drive paths and ordinary UNC paths only. Work with the
// expanded path so dot segments and extra separators cannot hide a root.
function NormalizeInstallDir(var dir: String): Boolean;
var
  i: Integer;
  drive, tail: String;
begin
  Result := False;
  dir := Trim(dir);
  StringChangeEx(dir, '/', '\', True);
  drive := Uppercase(Copy(dir, 1, 1));
  if Copy(dir, 1, 2) = '\\' then begin
    if (Copy(dir, 3, 1) = '\') or (Copy(dir, 3, 2) = '.\') then exit;
    tail := Copy(dir, 3, Length(dir));
  end else begin
    if (Length(dir) < 3) or (drive < 'A') or (drive > 'Z') or
       (Copy(dir, 2, 2) <> ':\') then exit;
    tail := Copy(dir, 4, Length(dir));
  end;
  for i := 1 to Length(tail) do
    if (Ord(tail[i]) < 32) or (Pos(tail[i], ':*?"<>|') > 0) then exit;
  dir := RemoveBackslashUnlessRoot(ExpandFileName(dir));
  drive := ExtractFileDrive(dir);
  if (drive = '') or (CompareText(AddBackslash(dir), AddBackslash(drive)) = 0) then exit;

  // Win32 can treat trailing dots and spaces as aliases for another directory.
  tail := AddBackslash(Copy(dir, Length(drive) + 1, Length(dir)));
  if (Pos('.\', tail) > 0) or (Pos(' \', tail) > 0) then exit;
  Result := True;
end;

// Keep a valid custom directory only when it belongs to legacy BitChord.
function DefaultDir(Param: String): String;
var
  root: Integer;
  path, location: String;
begin
  Result := ExpandConstant('{autopf}\{#AppName}');
  if not FindLegacyRegistration(root, path) then exit;
  if RegQueryStringValue(root, path, 'InstallLocation', location) and
     NormalizeInstallDir(location) then
    Result := location;
end;

// Read our install location before setup runs its uninstaller.
function InitializeSetup(): Boolean;
var
  root: Integer;
  path, location: String;
begin
  PrevDir := '';
  if FindUninstallValue(OurAppId, 'InstallLocation', root, path, location) then
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

function GetFileAttributes(name: String): LongWord;
  external 'GetFileAttributesW@kernel32.dll stdcall';

procedure WipeLeftovers(dir: String);
var
  parent, next, root: String;
  attributes: LongWord;
begin
  if dir = '' then exit;
  if not NormalizeInstallDir(dir) then begin
    Log('Skipping leftover cleanup: unsupported install path or root.');
    exit;
  end;
  if not DirExists(dir) then exit;
  // The wildcard passed to DelTree would follow a junction in the base path.
  parent := dir;
  root := AddBackslash(ExtractFileDrive(dir));
  while CompareText(AddBackslash(parent), root) <> 0 do begin
    attributes := GetFileAttributes(parent);
    if (attributes = $FFFFFFFF) or ((attributes and $400) <> 0) then begin
      Log('Skipping leftover cleanup: cannot safely traverse ' + parent);
      exit;
    end;
    next := ExtractFileDir(parent);
    if (next = '') or (next = parent) then exit;
    parent := next;
  end;
  if DelTree(AddBackslash(dir) + '*', False, True, True) then
    Log('Removed an install''s leftover files at ' + dir)
  else
    Log('Could not remove all leftover files at ' + dir);
end;

// The uninstaller can erase its registration, so capture its directory first.
procedure RemoveLegacyInstall;
var
  root: Integer;
  path, unins, location: String;
begin
  if not FindLegacyRegistration(root, path) then exit;
  if not RegQueryStringValue(root, path, 'QuietUninstallString', unins) or (unins = '') then
    if not RegQueryStringValue(root, path, 'UninstallString', unins) or (unins = '') then begin
      Log('No uninstaller for the old install under the shared AppId; skipping.');
      exit;
    end;
  if not RegQueryStringValue(root, path, 'InstallLocation', location) then
    location := '';
  RunUninstaller(unins);
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

// Inno Setup 6 runs this script in a 32-bit process on every target.
function CredEnumerate(filter, flags: LongWord; var count, entries: LongWord): Integer;
  external 'CredEnumerateW@advapi32.dll stdcall';
function CredDelete(target: String; kind, flags: LongWord): Integer;
  external 'CredDeleteW@advapi32.dll stdcall';
procedure CredFree(entries: LongWord);
  external 'CredFree@advapi32.dll stdcall';
function ReadCredentialWord(process: Integer; address: LongWord; var value: LongWord;
  size, bytesRead: LongWord): Integer;
  external 'ReadProcessMemory@kernel32.dll stdcall';
function CopyCredentialTarget(dest: String; source: LongWord; limit: Integer): LongWord;
  external 'lstrcpynW@kernel32.dll stdcall';

procedure ClearBitChordCredentials;
var
  count, entries, i, entry, kind, namePtr: LongWord;
  target: String;
begin
  if CredEnumerate(0, 0, count, entries) = 0 then begin
    if DLLGetLastError <> CredentialNotFound then
      Log('Could not enumerate Windows credentials for the BitChord reset.');
    exit;
  end;
  try
    for i := 1 to count do begin
      // CREDENTIALW starts with Flags, Type and TargetName. The keyring crate
      // uses "user.service", with vault parts named "user #2", "user #3", etc.
      if (ReadCredentialWord(-1, entries + (i - 1) * 4, entry, 4, 0) = 0) or
         (ReadCredentialWord(-1, entry + 4, kind, 4, 0) = 0) or
         (ReadCredentialWord(-1, entry + 8, namePtr, 4, 0) = 0) then begin
        Log('Could not read a Windows credential entry during the BitChord reset.');
        continue;
      end;
      if kind <> GenericCredential then continue;
      SetLength(target, MaxCredentialTargetLength + 1);
      if CopyCredentialTarget(target, namePtr, MaxCredentialTargetLength + 1) = 0 then begin
        Log('Could not read a Windows credential target during the BitChord reset.');
        continue;
      end;
      SetLength(target, Pos(#0, target) - 1);
      if Length(target) <= Length(CredentialServiceSuffix) then continue;
      if CompareText(Copy(target, Length(target) - Length(CredentialServiceSuffix) + 1,
         Length(CredentialServiceSuffix)), CredentialServiceSuffix) <> 0 then continue;
      if CredDelete(target, kind, 0) = 0 then
        if DLLGetLastError <> CredentialNotFound then
          Log('Could not remove a BitChord credential during the reset.');
    end;
  finally
    CredFree(entries);
  end;
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
    if DirExists(ExpandConstant('{userappdata}\bitchord')) then begin
      if DelTree(ExpandConstant('{userappdata}\bitchord'), True, True, True) then
        Log('Removed the previous BitChord data (%AppData%\bitchord) with the old install.')
      else
        Log('Could not remove all previous BitChord data.');
    end;
    ClearBitChordCredentials();
  end;
end;
