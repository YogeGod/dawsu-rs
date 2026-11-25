; dawsu! Windows Per-User Installer (Inno Setup 6)
;
; Installs into %LOCALAPPDATA%\Programs\dawsu by default — no admin rights required.
; Override defaults from tools/package-release.ps1 via:
;   ISCC.exe /DMyAppVersion=0.1.0 /DSourceDir=..\dist\stage\windows-x64 /DOutputDir=..\dist\release installer\dawsu.iss

#ifndef MyAppVersion
  #define MyAppVersion "0.1.0"
#endif

#ifndef SourceDir
  #define SourceDir "..\dist\stage\windows-x64"
#endif

#ifndef OutputDir
  #define OutputDir "..\dist\release"
#endif

#define MyAppName "dawsu!"
#define MyAppPublisher "YogeGod"
#define MyAppURL "https://github.com/YogeGod/dawsu-rs"
#define MyAppExeName "dawsu-tauri.exe"

[Setup]
AppId={{8E5F2D61-4C8B-4B97-9E71-3F5D8A9E42C1}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}/issues
AppUpdatesURL={#MyAppURL}/releases
DefaultDirName={autopf}\dawsu
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename=dawsu-v{#MyAppVersion}-windows-x64-setup
SetupIconFile=..\apps\tauri\icons\icon.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
UninstallDisplayName={#MyAppName}
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
ChangesAssociations=yes
CloseApplications=force
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "assoc_osu"; Description: "Associate .osu beatmap files with {#MyAppName}"; GroupDescription: "File associations (optional):"; Flags: unchecked
Name: "assoc_osz"; Description: "Associate .osz mapset archives with {#MyAppName}"; GroupDescription: "File associations (optional):"; Flags: unchecked

[Files]
Source: "{#SourceDir}\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\dawsu_vst3_host.exe"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "{#SourceDir}\THIRD-PARTY-NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "{#SourceDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "{#SourceDir}\assets\*"; DestDir: "{app}\assets"; Flags: ignoreversion recursesubdirs createallsubdirs skipifsourcedoesntexist

[Icons]
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{autoprograms}\Uninstall {#MyAppName}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Registry]
; Optional per-user .osu file association (HKA resolves to HKCU in per-user mode)
Root: HKA; Subkey: "Software\Classes\.osu\OpenWithProgids"; ValueType: string; ValueName: "dawsu.Beatmap"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc_osu
Root: HKA; Subkey: "Software\Classes\dawsu.Beatmap"; ValueType: string; ValueName: ""; ValueData: "osu! Beatmap (dawsu!)"; Flags: uninsdeletekey; Tasks: assoc_osu
Root: HKA; Subkey: "Software\Classes\dawsu.Beatmap\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"",0"; Tasks: assoc_osu
Root: HKA; Subkey: "Software\Classes\dawsu.Beatmap\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""; Tasks: assoc_osu

; Optional per-user .osz file association
Root: HKA; Subkey: "Software\Classes\.osz\OpenWithProgids"; ValueType: string; ValueName: "dawsu.Mapset"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc_osz
Root: HKA; Subkey: "Software\Classes\dawsu.Mapset"; ValueType: string; ValueName: ""; ValueData: "osu! Mapset Archive (dawsu!)"; Flags: uninsdeletekey; Tasks: assoc_osz
Root: HKA; Subkey: "Software\Classes\dawsu.Mapset\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"",0"; Tasks: assoc_osz
Root: HKA; Subkey: "Software\Classes\dawsu.Mapset\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""; Tasks: assoc_osz

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent
; Silent auto-update relaunch triggered by apps/tauri/src/updater.rs when /relaunch=1 is passed.
Filename: "{app}\{#MyAppExeName}"; Flags: nowait; Check: Relaunch

[UninstallDelete]
Type: files; Name: "{app}\*.old"

[Code]
function Relaunch: Boolean;
begin
  Result := WizardSilent and (ExpandConstant('{param:relaunch|0}') = '1');
end;
