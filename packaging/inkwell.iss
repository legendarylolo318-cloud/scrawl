; Inno Setup script. Build: iscc packaging\inkwell.iss  (expects dist\inkwell.exe)
#define AppVersion GetEnv("INKWELL_VERSION")
[Setup]
AppName=Inkwell
AppVersion={#AppVersion}
DefaultDirName={autopf}\Inkwell
DefaultGroupName=Inkwell
OutputDir=..\dist
OutputBaseFilename=Inkwell-{#AppVersion}-setup
Compression=lzma2
SolidCompression=yes
ArchitecturesInstallIn64BitMode=x64compatible
SetupIconFile=inkwell.ico
UninstallDisplayIcon={app}\inkwell.exe
[Files]
Source: "..\dist\inkwell.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
[Icons]
Name: "{group}\Inkwell"; Filename: "{app}\inkwell.exe"
Name: "{autodesktop}\Inkwell"; Filename: "{app}\inkwell.exe"; Tasks: desktopicon
[Tasks]
Name: "desktopicon"; Description: "Create a desktop icon"
[Run]
Filename: "{app}\inkwell.exe"; Description: "Launch Inkwell"; Flags: nowait postinstall skipifsilent
