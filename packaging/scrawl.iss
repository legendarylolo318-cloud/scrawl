; Inno Setup script. Build: iscc packaging\scrawl.iss  (expects dist\scrawl.exe)
#define AppVersion GetEnv("SCRAWL_VERSION")
[Setup]
AppName=Scrawl
AppVersion={#AppVersion}
DefaultDirName={autopf}\Scrawl
DefaultGroupName=Scrawl
OutputDir=..\dist
OutputBaseFilename=Scrawl-{#AppVersion}-windows-x64-setup
Compression=lzma2
SolidCompression=yes
ArchitecturesInstallIn64BitMode=x64compatible
SetupIconFile=scrawl.ico
UninstallDisplayIcon={app}\scrawl.exe
[Files]
Source: "..\dist\scrawl.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
[Icons]
Name: "{group}\Scrawl"; Filename: "{app}\scrawl.exe"
Name: "{autodesktop}\Scrawl"; Filename: "{app}\scrawl.exe"; Tasks: desktopicon
[Tasks]
Name: "desktopicon"; Description: "Create a desktop icon"
[Run]
Filename: "{app}\scrawl.exe"; Description: "Launch Scrawl"; Flags: nowait postinstall skipifsilent
