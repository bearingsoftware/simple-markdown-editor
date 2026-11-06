; Per-user installer for Simple Markdown Editor (MDEDIT-137). Never elevates: AGENTS.md, "Windows
; integration". Built by .github/workflows/package.yml:
;   ISCC.exe /DAppVersion=0.1.0 /DSourceDir=<dir holding mdedit.exe, LICENSE, THIRD-PARTY-NOTICES.txt, README.md> mdedit.iss

#ifndef AppVersion
  #error AppVersion must be passed with /DAppVersion=
#endif
#ifndef SourceDir
  #error SourceDir must be passed with /DSourceDir=
#endif

[Setup]
; Never change: it is how upgrades find the install.
AppId={{6B1E2F84-3C5A-4D97-A0E8-7F2C9D41B365}
AppName=Simple Markdown Editor
AppVersion={#AppVersion}
AppPublisher=Bearing Software
AppPublisherURL=https://github.com/bearingsoftware/simple-markdown-editor
AppSupportURL=https://github.com/bearingsoftware/simple-markdown-editor/issues
DefaultDirName={localappdata}\Programs\Simple Markdown Editor
DefaultGroupName=Simple Markdown Editor
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputBaseFilename=mdedit-{#AppVersion}-windows-x64-setup
SetupIconFile=..\..\..\..\assets\icons\mdedit.ico
UninstallDisplayIcon={app}\mdedit.exe
LicenseFile={#SourceDir}\LICENSE
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop icon"; Flags: unchecked

[Files]
Source: "{#SourceDir}\mdedit.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\THIRD-PARTY-NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Simple Markdown Editor"; Filename: "{app}\mdedit.exe"
Name: "{autodesktop}\Simple Markdown Editor"; Filename: "{app}\mdedit.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\mdedit.exe"; Parameters: "--register"; Flags: runhidden waituntilterminated
Filename: "{app}\mdedit.exe"; Description: "Launch Simple Markdown Editor"; Flags: postinstall nowait skipifsilent

[UninstallRun]
Filename: "{app}\mdedit.exe"; Parameters: "--unregister"; Flags: runhidden waituntilterminated; RunOnceId: "UnregisterShell"
