#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{A2BCE3F2-3BF9-4B20-9EC2-2A090D82C5B7}
AppName=Lantern
AppVersion={#AppVersion}
AppPublisher=Cheter
DefaultDirName={autopf}\Showtime Suite\Lantern
DefaultGroupName=Lantern
DisableProgramGroupPage=yes
OutputDir=..\..\dist
OutputBaseFilename=Lantern-Setup-{#AppVersion}
SetupIconFile=..\..\assets\icon.ico
UninstallDisplayIcon={app}\lantern.exe
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "german"; MessagesFile: "compiler:Languages\German.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "..\..\stage\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs

[Icons]
Name: "{autoprograms}\Lantern"; Filename: "{app}\lantern.exe"
Name: "{autodesktop}\Lantern"; Filename: "{app}\lantern.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\lantern.exe"; Description: "{cm:LaunchProgram,Lantern}"; Flags: nowait postinstall
