; Inno Setup script for the NexPanel installer. Built by scripts/package.ps1:
;   ISCC /DMyAppVersion=X.Y.Z /DSourceDir=<stage> /DOutputDir=<dist> installer.iss
; Per-user install (no admin) into the same layout as scripts/install.ps1, so the auto-updater
; accepts it: it needs `.nexpanel-install.json` next to nexpanel.exe (written in [Code] below).
#ifndef MyAppVersion
  #error MyAppVersion is not defined
#endif
#ifndef SourceDir
  #error SourceDir is not defined
#endif
#ifndef OutputDir
  #define OutputDir "..\dist"
#endif

[Setup]
; Fixed id: lets a new installer upgrade the previous installation.
AppId={{6C1D3B0E-5B7A-4C0B-9E44-7A2F0D5E8A31}
AppName=NexPanel
AppVersion={#MyAppVersion}
AppVerName=NexPanel {#MyAppVersion}
AppPublisher=NexPanel
DefaultDirName={localappdata}\NexPanel\bin
DisableDirPage=yes
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.22000
OutputDir={#OutputDir}
OutputBaseFilename=nexpanel-{#MyAppVersion}-windows-x64-setup
UninstallDisplayName=NexPanel
UninstallDisplayIcon={app}\nexpanel.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
; Single-instance mutex of the app: setup asks to close a running NexPanel instead of failing.
AppMutex=Local\NexPanel.Panel
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "german"; MessagesFile: "compiler:Languages\German.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\nexpanel.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\THIRD-PARTY-NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\VERSION.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\NexPanel"; Filename: "{app}\nexpanel.exe"; WorkingDir: "{app}"; Comment: "NexPanel – Windows Control Center"
Name: "{userdesktop}\NexPanel"; Filename: "{app}\nexpanel.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\nexpanel.exe"; Description: "{cm:LaunchProgram,NexPanel}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; Files the auto-updater adds or leaves behind. settings.json and foreign files stay untouched.
Type: files; Name: "{app}\.nexpanel-install.json"
Type: files; Name: "{app}\install.ps1"
Type: files; Name: "{app}\uninstall.ps1"
Type: files; Name: "{app}\*.old"
Type: files; Name: "{app}\*.new"
Type: dirifempty; Name: "{app}"

[Code]
function JsonEscape(const S: String): String;
begin
  Result := S;
  StringChangeEx(Result, '\', '\\', True);
  StringChangeEx(Result, '"', '\"', True);
end;

procedure WriteMarker;
var
  Lines: TArrayOfString;
  Dir: String;
begin
  Dir := RemoveBackslash(ExpandConstant('{app}'));
  SetArrayLength(Lines, 1);
  Lines[0] := '{"product":"NexPanel","directory":"' + JsonEscape(Dir) + '","version":"{#MyAppVersion}"}';
  if not SaveStringsToUTF8File(Dir + '\.nexpanel-install.json', Lines, False) then
    RaiseException('Installationsmarkierung konnte nicht geschrieben werden.');
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
    WriteMarker;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Value, Expected: String;
begin
  if CurUninstallStep = usUninstall then
  begin
    { Remove the autostart entry only if it points at this installation. }
    Expected := '"' + ExpandConstant('{app}\nexpanel.exe') + '" --background';
    if RegQueryStringValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'NexPanel', Value) then
      if CompareText(Trim(Value), Expected) = 0 then
        RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'NexPanel');
  end;
end;
