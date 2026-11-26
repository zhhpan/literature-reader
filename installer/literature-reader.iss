#ifndef AppVersion
  #error AppVersion must be defined by build.ps1
#endif
#ifndef PayloadDir
  #error PayloadDir must be defined by build.ps1
#endif

#define NodeDirectory "node-v22.23.5-win-x64"
#define PopplerDirectory "poppler-26.09.0"
#define PopplerManifest PopplerDirectory + ".sha256"
#define PopplerBinSource "{tmp}\reader-deps\" + PopplerDirectory + "\Library\bin"
#define PopplerBinTarget "{app}\runtime\poppler\Library\bin"
#define SdkVendorRoot "\@tencent-ai\agent-sdk\cli\vendor"
#define SdkVendor "\node_modules" + SdkVendorRoot

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
SetupIconFile=..\assets\reader.ico
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
Source: "{#PopplerManifest}"; Flags: dontcopy
Source: "{#PayloadDir}\reader-bridge-launcher.sha256"; Flags: dontcopy
Source: "{#PayloadDir}\reader-bridge-sdk.sha256"; Flags: dontcopy
Source: "{#PayloadDir}\literature-reader.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\runtime\onnxruntime.dll"; DestDir: "{app}\runtime"; Flags: ignoreversion skipifsourcedoesntexist
Source: "{#PayloadDir}\runtime\DirectML.dll"; DestDir: "{app}\runtime"; Flags: ignoreversion skipifsourcedoesntexist
Source: "{#PayloadDir}\runtime\layout\doclayout.onnx"; DestDir: "{app}\runtime\layout"; Flags: ignoreversion skipifsourcedoesntexist
Source: "{#PayloadDir}\bridge\bridge.mjs"; DestDir: "{app}\bridge"; Flags: ignoreversion; Check: NeedsBridgeLauncher
Source: "{#PayloadDir}\bridge\package.json"; DestDir: "{app}\bridge"; Flags: ignoreversion; Check: NeedsBridgeLauncher
Source: "{#PayloadDir}\bridge\package-lock.json"; DestDir: "{app}\bridge"; Flags: ignoreversion; Check: NeedsBridgeLauncher
; The SDK selects its native helpers by Node.js platform and architecture.
Source: "{#PayloadDir}\bridge\node_modules\*"; DestDir: "{app}\bridge\node_modules"; Excludes: "\.package-lock.json,{#SdkVendorRoot}\ripgrep\*-darwin\*,{#SdkVendorRoot}\ripgrep\*-linux\*,{#SdkVendorRoot}\genie-trash\darwin-*,{#SdkVendorRoot}\genie-trash\linux-*"; Flags: ignoreversion recursesubdirs createallsubdirs solidbreak; Check: NeedsBridgeSdk
Source: "{#PayloadDir}\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion recursesubdirs createallsubdirs solidbreak

; PrepareToInstall verifies upstream archives before copying these files.
Source: "{tmp}\reader-deps\{#NodeDirectory}\node.exe"; DestDir: "{app}\runtime\node"; ExternalSize: 90_000_000; Flags: external ignoreversion; Check: NeedsNodeRuntime
Source: "{tmp}\reader-deps\{#NodeDirectory}\LICENSE"; DestDir: "{app}\runtime\node"; ExternalSize: 150_000; Flags: external ignoreversion; Check: NeedsNodeRuntime
Source: "{#PopplerBinSource}\pdfinfo.exe"; DestDir: "{#PopplerBinTarget}"; ExternalSize: 100_000; Flags: external ignoreversion; Check: NeedsPopplerRuntime
Source: "{#PopplerBinSource}\pdfimages.exe"; DestDir: "{#PopplerBinTarget}"; ExternalSize: 100_000; Flags: external ignoreversion; Check: NeedsPopplerRuntime
Source: "{#PopplerBinSource}\pdftohtml.exe"; DestDir: "{#PopplerBinTarget}"; ExternalSize: 250_000; Flags: external ignoreversion; Check: NeedsPopplerRuntime
Source: "{#PopplerBinSource}\pdftoppm.exe"; DestDir: "{#PopplerBinTarget}"; ExternalSize: 100_000; Flags: external ignoreversion; Check: NeedsPopplerRuntime
Source: "{#PopplerBinSource}\pdftotext.exe"; DestDir: "{#PopplerBinTarget}"; ExternalSize: 100_000; Flags: external ignoreversion; Check: NeedsPopplerRuntime
Source: "{#PopplerBinSource}\*.dll"; DestDir: "{#PopplerBinTarget}"; ExternalSize: 115_000_000; Flags: external ignoreversion; Check: NeedsPopplerRuntime
; Keep runtime data, configuration, and license files. Omit development files and extra tools.
Source: "{tmp}\reader-deps\{#PopplerDirectory}\*"; DestDir: "{app}\runtime\poppler"; Excludes: "\Library\bin\*,\Library\include\*,\Library\lib\*.lib,\Library\lib\pkgconfig\*,\Library\share\man\*"; ExternalSize: 20_000_000; Flags: external ignoreversion recursesubdirs; Check: NeedsPopplerRuntime

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
Filename: "{app}\literature-reader.exe"; WorkingDir: "{app}"; Flags: postinstall nowait skipifnotsilent; Check: IsReaderUpdate
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
  SynchronizeAccess = $00100000;
  ErrorInvalidParameter = 87;
  WaitObject0 = 0;
  CompareEqual = 2;

var
  DownloadPage: TDownloadWizardPage;
  ReuseNodeRuntime: Boolean;
  ReusePopplerRuntime: Boolean;
  ReuseBridgeLauncher: Boolean;
  ReuseBridgeSdk: Boolean;

function OpenReaderProcess(Access: Cardinal; InheritHandle: LongInt; ProcessId: Cardinal): THandle;
external 'OpenProcess@kernel32.dll stdcall';
function WaitForReaderProcess(ProcessHandle: THandle; Timeout: Cardinal): Cardinal;
external 'WaitForSingleObject@kernel32.dll stdcall';
function CloseReaderProcess(ProcessHandle: THandle): LongInt;
external 'CloseHandle@kernel32.dll stdcall';
function CompareReaderPaths(Left: String; LeftLength: LongInt; Right: String; RightLength: LongInt;
  IgnoreCase: LongInt): LongInt;
external 'CompareStringOrdinal@kernel32.dll stdcall';

function IsReaderUpdate: Boolean;
begin
  Result := ExpandConstant('{param:READERUPDATE|0}') = '1';
end;

function ShouldSkipPage(PageID: Integer): Boolean;
begin
  Result := (PageID = wpSelectDir) and IsReaderUpdate;
end;

function NormalReaderPath(Path: String): String;
begin
  Result := Path;
  if Copy(Path, 1, 8) = '\\?\UNC\' then
    Result := '\\' + Copy(Path, 9, Length(Path) - 8)
  else if Copy(Path, 1, 4) = '\\?\' then
    Delete(Result, 1, 4);
end;

function WaitForReaderToClose: Boolean;
var
  ReaderPid: Int64;
  ReaderProcess: THandle;
begin
  Result := False;
  ReaderPid := StrToInt64Def(ExpandConstant('{param:READERPID|0}'), 0);
  if (ReaderPid <= 0) or (ReaderPid > $FFFFFFFF) then
    Exit;
  ReaderProcess := OpenReaderProcess(SynchronizeAccess, 0, Cardinal(ReaderPid));
  if ReaderProcess = 0 then
  begin
    { OpenProcess reports this error when the reader has already exited. }
    Result := DLLGetLastError = ErrorInvalidParameter;
    Exit;
  end;
  try
    Result := WaitForReaderProcess(ReaderProcess, 15000) = WaitObject0;
  finally
    CloseReaderProcess(ReaderProcess);
  end;
end;

function NeedsNodeRuntime: Boolean;
begin
  Result := not ReuseNodeRuntime;
end;

function NeedsPopplerRuntime: Boolean;
begin
  Result := not ReusePopplerRuntime;
end;

function NeedsBridgeSdk: Boolean;
begin
  Result := not ReuseBridgeSdk;
end;

function NeedsBridgeLauncher: Boolean;
begin
  Result := not ReuseBridgeLauncher;
end;

function MatchesDependencyManifest(Directory: String; ManifestName: String; ArchiveHash: String): Boolean;
var
  ManifestPath: String;
  Entries: TArrayOfString;
  FirstEntry: Integer;
  Index: Integer;
  ExpectedHash: String;
  RelativePath: String;
  FilePath: String;
begin
  Result := False;
  if not DirExists(Directory) then
    Exit;
  try
    ManifestPath := ExpandConstant('{tmp}\') + ManifestName;
    if not FileExists(ManifestPath) then
      ExtractTemporaryFile(ManifestName);
    if not LoadStringsFromFile(ManifestPath, Entries) or (GetArrayLength(Entries) = 0) then
      Exit;
    FirstEntry := 0;
    if ArchiveHash <> '' then
    begin
      if CompareText(Entries[0], ArchiveHash) <> 0 then
        Exit;
      FirstEntry := 1;
    end;
    if GetArrayLength(Entries) <= FirstEntry then
      Exit;
    for Index := FirstEntry to GetArrayLength(Entries) - 1 do
    begin
      if (Length(Entries[Index]) < 66) or (Entries[Index][65] <> ' ') then
        Exit;
      ExpectedHash := Copy(Entries[Index], 1, 64);
      RelativePath := Copy(Entries[Index], 66, Length(Entries[Index]));
      if (RelativePath[1] = '\') or (Pos(':', RelativePath) <> 0) or
         (Pos('/', RelativePath) <> 0) or (Pos('\..\', '\' + RelativePath + '\') <> 0) then
        Exit;
      FilePath := AddBackslash(Directory) + RelativePath;
      if not FileExists(FilePath) then
        Exit;
      if CompareText(GetSHA256OfFile(FilePath), ExpectedHash) <> 0 then
        Exit;
    end;
    Result := True;
  except
    Log('Could not verify the installed dependency files against ' + ManifestName + '.');
  end;
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
  ReaderExe: String;
  InstalledExe: String;
  Staging: String;
  PopplerBin: String;
begin
  Result := '';
  if IsReaderUpdate and WizardSilent then
  begin
    ReaderExe := NormalReaderPath(ExpandConstant('{param:READEREXE|}'));
    InstalledExe := NormalReaderPath(ExpandConstant('{app}\literature-reader.exe'));
    if (CompareReaderPaths(ReaderExe, -1, InstalledExe, -1, 1) <> CompareEqual) or
       not FileExists(ReaderExe) then
    begin
      Result := 'Setup could not identify the existing installation folder. Run the installer manually.';
      Exit;
    end;
    if not WaitForReaderToClose then
    begin
      Result := 'Setup could not confirm that Literature Reader has closed. Open the reader and try updating again.';
      Exit;
    end;
  end;
  Staging := ExpandConstant('{tmp}\reader-deps');
  ReuseNodeRuntime := HasPinnedNodeRuntime;
  ReusePopplerRuntime := MatchesDependencyManifest(ExpandConstant('{app}\runtime\poppler'),
    '{#PopplerManifest}', PopplerHash);
  ReuseBridgeLauncher := MatchesDependencyManifest(ExpandConstant('{app}\bridge'),
    'reader-bridge-launcher.sha256', '');
  ReuseBridgeSdk := MatchesDependencyManifest(ExpandConstant('{app}\bridge\node_modules'),
    'reader-bridge-sdk.sha256', '');
  if ReuseNodeRuntime and ReusePopplerRuntime then
    Exit;
  DownloadPage.Clear;
  if NeedsNodeRuntime then
    DownloadPage.Add(NodeUrl, 'node.7z', NodeHash);
  if NeedsPopplerRuntime then
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
      if NeedsPopplerRuntime then
      begin
        ExtractArchive(ExpandConstant('{tmp}\poppler.zip'), Staging, '', True, nil);
        PopplerBin := Staging + '\{#PopplerDirectory}\Library\bin\';
        if not FileExists(PopplerBin + 'pdfinfo.exe') or
           not FileExists(PopplerBin + 'pdfimages.exe') or
           not FileExists(PopplerBin + 'pdftohtml.exe') or
           not FileExists(PopplerBin + 'pdftoppm.exe') or
           not FileExists(PopplerBin + 'pdftotext.exe') then
          RaiseException('The verified Poppler archive did not contain the expected runtime files.');
      end;
    except
      Result := 'Setup could not prepare the required components.' + #13#10 + GetExceptionMessage;
    end;
  finally
    DownloadPage.Hide;
  end;
end;
