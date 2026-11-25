#ifndef AppVersion
  #error AppVersion must be defined by build.ps1
#endif
#ifndef PayloadDir
  #error PayloadDir must be defined by build.ps1
#endif

#define NodeDirectory "node-v22.23.5-win-x64"
#define PopplerDirectory "poppler-26.09.0"

[Setup]
AppId={{9B509341-8C39-4166-AEA1-25FB747E2231}
AppName=Literature Reader
AppVersion={#AppVersion}
AppPublisher=Literature Reader contributors
VersionInfoVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\Literature Reader
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
UninstallDisplayIcon={app}\literature-reader.exe
RestartApplications=no
Compression=lzma2/normal
SolidCompression=yes
ArchiveExtraction=full
WizardStyle=modern
OutputDir=..\target\release
OutputBaseFilename=LiteratureReader-Setup-{#AppVersion}-x64

[Tasks]
Name: desktopicon; Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
Source: "{#PayloadDir}\literature-reader.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\bridge\*"; DestDir: "{app}\bridge"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#PayloadDir}\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion recursesubdirs createallsubdirs

; PrepareToInstall verifies and unpacks both upstream archives before these copies.
Source: "{tmp}\reader-deps\{#NodeDirectory}\node.exe"; DestDir: "{app}\runtime\node"; ExternalSize: 90_000_000; Flags: external ignoreversion
Source: "{tmp}\reader-deps\{#NodeDirectory}\LICENSE"; DestDir: "{app}\runtime\node"; ExternalSize: 150_000; Flags: external ignoreversion
Source: "{tmp}\reader-deps\{#PopplerDirectory}\*"; DestDir: "{app}\runtime\poppler"; ExternalSize: 150_000_000; Flags: external ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\Literature Reader"; Filename: "{app}\literature-reader.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\Literature Reader"; Filename: "{app}\literature-reader.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\literature-reader.exe"; Description: "Open Literature Reader"; WorkingDir: "{app}"; Flags: postinstall skipifsilent unchecked

[Code]
const
  NodeUrl = 'https://nodejs.org/dist/v22.23.5/node-v22.23.5-win-x64.zip';
  NodeHash = '4a191082e9706866830379d577d2cc8a50c776fcd9436a8d061b063ed0d9a0e3';
  PopplerUrl = 'https://github.com/oschwartz10612/poppler-windows/releases/download/v26.09.0-0/Release-26.09.0-0.zip';
  PopplerHash = '7a6f256a0ddf7536182246a5733331bf4677cbcc34f4663774947ad34556c8d0';

var
  DownloadPage: TDownloadWizardPage;

procedure InitializeWizard;
begin
  DownloadPage := CreateDownloadPage('Download required components',
    'Setup downloads Node.js and Poppler and verifies their SHA-256 hashes.', nil);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Staging: String;
  PopplerBin: String;
begin
  Result := '';
  Staging := ExpandConstant('{tmp}\reader-deps');
  DownloadPage.Clear;
  DownloadPage.Add(NodeUrl, 'node.zip', NodeHash);
  DownloadPage.Add(PopplerUrl, 'poppler.zip', PopplerHash);
  DownloadPage.Show;
  try
    try
      DownloadPage.Download;
      if not ForceDirectories(Staging) then
        RaiseException('Unable to create the dependency staging directory.');
      ExtractArchive(ExpandConstant('{tmp}\node.zip'), Staging, '', True, nil);
      ExtractArchive(ExpandConstant('{tmp}\poppler.zip'), Staging, '', True, nil);

      PopplerBin := Staging + '\{#PopplerDirectory}\Library\bin\';
      if not FileExists(Staging + '\{#NodeDirectory}\node.exe') or
         not FileExists(Staging + '\{#NodeDirectory}\LICENSE') or
         not FileExists(PopplerBin + 'pdfinfo.exe') or
         not FileExists(PopplerBin + 'pdfimages.exe') or
         not FileExists(PopplerBin + 'pdftohtml.exe') or
         not FileExists(PopplerBin + 'pdftoppm.exe') or
         not FileExists(PopplerBin + 'pdftotext.exe') then
        RaiseException('The verified archives did not contain the expected runtime files.');
    except
      Result := 'Setup could not prepare the required components.' + #13#10 + GetExceptionMessage;
    end;
  finally
    DownloadPage.Hide;
  end;
end;
