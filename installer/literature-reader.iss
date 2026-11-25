#ifndef AppVersion
  #error AppVersion must be defined by build.ps1
#endif
#ifndef PayloadDir
  #error PayloadDir must be defined by build.ps1
#endif

#define NodeDirectory "node-v22.23.5-win-x64"
#define PopplerDirectory "poppler-26.09.0"
#define PopplerBinSource "{tmp}\reader-deps\" + PopplerDirectory + "\Library\bin"
#define PopplerBinTarget "{app}\runtime\poppler\Library\bin"
#define SdkVendor "\node_modules\@tencent-ai\agent-sdk\cli\vendor"

[Setup]
AppId={{9B509341-8C39-4166-AEA1-25FB747E2231}
AppName=Literature Reader
AppVersion={#AppVersion}
AppPublisher=Literature Reader contributors
VersionInfoVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\Literature Reader
UsePreviousAppDir=yes
DisableDirPage=no
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
OutputDir={#PayloadDir}\output
OutputBaseFilename=LiteratureReader-Setup-{#AppVersion}-x64
#ifdef ReleaseSigning
SignTool=readerRelease
SignedUninstaller=yes
SignedUninstallerDir={#PayloadDir}
#endif

[Tasks]
Name: desktopicon; Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
Source: "{#PayloadDir}\literature-reader.exe"; DestDir: "{app}"; Flags: ignoreversion
; The SDK selects its native helpers by Node.js platform and architecture.
Source: "{#PayloadDir}\bridge\*"; DestDir: "{app}\bridge"; Excludes: "{#SdkVendor}\ripgrep\*-darwin\*,{#SdkVendor}\ripgrep\*-linux\*,{#SdkVendor}\genie-trash\darwin-*,{#SdkVendor}\genie-trash\linux-*"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#PayloadDir}\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion recursesubdirs createallsubdirs

; PrepareToInstall verifies upstream archives before copying these files.
Source: "{tmp}\reader-deps\{#NodeDirectory}\node.exe"; DestDir: "{app}\runtime\node"; ExternalSize: 90_000_000; Flags: external ignoreversion; Check: NeedsNodeRuntime
Source: "{tmp}\reader-deps\{#NodeDirectory}\LICENSE"; DestDir: "{app}\runtime\node"; ExternalSize: 150_000; Flags: external ignoreversion; Check: NeedsNodeRuntime
Source: "{#PopplerBinSource}\pdfinfo.exe"; DestDir: "{#PopplerBinTarget}"; ExternalSize: 100_000; Flags: external ignoreversion
Source: "{#PopplerBinSource}\pdfimages.exe"; DestDir: "{#PopplerBinTarget}"; ExternalSize: 100_000; Flags: external ignoreversion
Source: "{#PopplerBinSource}\pdftohtml.exe"; DestDir: "{#PopplerBinTarget}"; ExternalSize: 250_000; Flags: external ignoreversion
Source: "{#PopplerBinSource}\pdftoppm.exe"; DestDir: "{#PopplerBinTarget}"; ExternalSize: 100_000; Flags: external ignoreversion
Source: "{#PopplerBinSource}\pdftotext.exe"; DestDir: "{#PopplerBinTarget}"; ExternalSize: 100_000; Flags: external ignoreversion
Source: "{#PopplerBinSource}\*.dll"; DestDir: "{#PopplerBinTarget}"; ExternalSize: 115_000_000; Flags: external ignoreversion
; Keep runtime data, configuration, and license files. Omit development files and extra tools.
Source: "{tmp}\reader-deps\{#PopplerDirectory}\*"; DestDir: "{app}\runtime\poppler"; Excludes: "\Library\bin\*,\Library\include\*,\Library\lib\*.lib,\Library\lib\pkgconfig\*,\Library\share\man\*"; ExternalSize: 20_000_000; Flags: external ignoreversion recursesubdirs

[InstallDelete]
; Clean up SDK binaries previously installed by the Windows package.
Type: files; Name: "{app}\bridge{#SdkVendor}\ripgrep\arm64-darwin\rg"
Type: files; Name: "{app}\bridge{#SdkVendor}\ripgrep\arm64-darwin\ripgrep.node"
Type: files; Name: "{app}\bridge{#SdkVendor}\ripgrep\arm64-linux\rg"
Type: files; Name: "{app}\bridge{#SdkVendor}\ripgrep\arm64-linux\ripgrep.node"
Type: files; Name: "{app}\bridge{#SdkVendor}\ripgrep\x64-darwin\rg"
Type: files; Name: "{app}\bridge{#SdkVendor}\ripgrep\x64-darwin\ripgrep.node"
Type: files; Name: "{app}\bridge{#SdkVendor}\ripgrep\x64-linux\rg"
Type: files; Name: "{app}\bridge{#SdkVendor}\ripgrep\x64-linux\ripgrep.node"
Type: files; Name: "{app}\bridge{#SdkVendor}\genie-trash\darwin-arm64"
Type: files; Name: "{app}\bridge{#SdkVendor}\genie-trash\darwin-x64"
Type: files; Name: "{app}\bridge{#SdkVendor}\genie-trash\linux-arm64"
Type: files; Name: "{app}\bridge{#SdkVendor}\genie-trash\linux-x64"

[Icons]
Name: "{autoprograms}\Literature Reader"; Filename: "{app}\literature-reader.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\Literature Reader"; Filename: "{app}\literature-reader.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\literature-reader.exe"; Description: "Reopen Literature Reader"; WorkingDir: "{app}"; Flags: postinstall nowait skipifsilent; Check: IsReaderUpdate
Filename: "{app}\literature-reader.exe"; Description: "Open Literature Reader"; WorkingDir: "{app}"; Flags: postinstall nowait skipifsilent unchecked; Check: not IsReaderUpdate

[Code]
const
  NodeUrl = 'https://nodejs.org/dist/v22.23.5/node-v22.23.5-win-x64.7z';
  NodeHash = 'aac7072c663c5efa2147aa7d51022d56efb1aa75c7dd0a3e7582501c692a045a';
  { SHA-256 of win-x64/node.exe in the signed Node.js v22.23.5 release checksums. }
  NodeExecutableHash = '247e9693d316da9857e44990e5e11709f596f18112251b8026ec6eed80bdf38a';
  PopplerUrl = 'https://github.com/oschwartz10612/poppler-windows/releases/download/v26.09.0-0/Release-26.09.0-0.zip';
  PopplerHash = '7a6f256a0ddf7536182246a5733331bf4677cbcc34f4663774947ad34556c8d0';

var
  DownloadPage: TDownloadWizardPage;
  ReuseNodeRuntime: Boolean;

function IsReaderUpdate: Boolean;
begin
  Result := ExpandConstant('{param:READERUPDATE|0}') = '1';
end;

function ShouldSkipPage(PageID: Integer): Boolean;
begin
  Result := (PageID = wpSelectDir) and IsReaderUpdate;
end;

function NeedsNodeRuntime: Boolean;
begin
  Result := not ReuseNodeRuntime;
end;

function HasPinnedNodeRuntime: Boolean;
var
  NodePath: String;
  NodeExecutable: String;
begin
  Result := False;
  NodePath := ExpandConstant('{app}\runtime\node');
  NodeExecutable := NodePath + '\node.exe';
  if FileExists(NodeExecutable) and FileExists(NodePath + '\LICENSE') then
    try
      Result := CompareText(GetSHA256OfFile(NodeExecutable), NodeExecutableHash) = 0;
    except
      Log('The installed Node.js runtime could not be verified.');
    end;
end;

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
  ReuseNodeRuntime := HasPinnedNodeRuntime;
  DownloadPage.Clear;
  if NeedsNodeRuntime then
    DownloadPage.Add(NodeUrl, 'node.7z', NodeHash);
  DownloadPage.Add(PopplerUrl, 'poppler.zip', PopplerHash);
  DownloadPage.Show;
  try
    try
      DownloadPage.Download;
      if not ForceDirectories(Staging) then
        RaiseException('Unable to create the dependency staging directory.');
      if NeedsNodeRuntime then
      begin
        ExtractArchive(ExpandConstant('{tmp}\node.7z'), Staging, '', True, nil);
        if not FileExists(Staging + '\{#NodeDirectory}\node.exe') or
           not FileExists(Staging + '\{#NodeDirectory}\LICENSE') then
          RaiseException('The verified Node.js archive did not contain the expected runtime files.');
      end;
      ExtractArchive(ExpandConstant('{tmp}\poppler.zip'), Staging, '', True, nil);

      PopplerBin := Staging + '\{#PopplerDirectory}\Library\bin\';
      if not FileExists(PopplerBin + 'pdfinfo.exe') or
         not FileExists(PopplerBin + 'pdfimages.exe') or
         not FileExists(PopplerBin + 'pdftohtml.exe') or
         not FileExists(PopplerBin + 'pdftoppm.exe') or
         not FileExists(PopplerBin + 'pdftotext.exe') then
        RaiseException('The verified Poppler archive did not contain the expected runtime files.');
    except
      Result := 'Setup could not prepare the required components.' + #13#10 + GetExceptionMessage;
    end;
  finally
    DownloadPage.Hide;
  end;
end;
