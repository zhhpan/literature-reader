#Requires -Version 5.1
[CmdletBinding()]
param(
    [string]$Compiler = 'ISCC.exe',
    [string]$AppExecutable,
    [switch]$Offline,
    [switch]$SkipLayout,
    [ValidateNotNullOrEmpty()]
    [ValidatePattern('^[0-9a-fA-F]{40}$')]
    [string]$SigningCertificateThumbprint,
    [string]$SignTool = 'signtool.exe',
    [ValidatePattern('^https?://[a-zA-Z0-9.-]+(:[0-9]+)?(/[a-zA-Z0-9._/-]*)?$')]
    [string]$TimestampUrl = 'http://timestamp.digicert.com'
)

function Assert-ReleaseSignature([string]$Path, [string]$Thumbprint) {
    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Thumbprint -ne $Thumbprint -or
        !$signature.TimeStamperCertificate) {
        throw "Expected a trusted, timestamped release signature on $(Split-Path -Leaf $Path)."
    }
}

$ErrorActionPreference = 'Stop'
$project = Split-Path -Parent $PSScriptRoot
$compilerPath = (Get-Command $Compiler -CommandType Application -ErrorAction Stop).Source
$release = Join-Path $project 'target\release'
if (!$AppExecutable) { $AppExecutable = Join-Path $release 'literature-reader.exe' }
$app = Get-Item -LiteralPath $AppExecutable -ErrorAction Stop
if ($app.PSIsContainer) { throw 'The reader executable must be a file.' }

$versionLine = Select-String -LiteralPath (Join-Path $project 'Cargo.toml') -Pattern '^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"\s*$'
if (@($versionLine).Count -ne 1) { throw 'Unable to determine the application version.' }
$appVersion = $versionLine.Matches[0].Groups[1].Value

$signing = ![string]::IsNullOrEmpty($SigningCertificateThumbprint)
if ($signing) {
    $certificatePath = "Cert:\CurrentUser\My\$SigningCertificateThumbprint"
    $certificate = Get-Item -LiteralPath $certificatePath -ErrorAction SilentlyContinue
    if (!$certificate -or !$certificate.HasPrivateKey) {
        throw 'The release signing certificate and its private key must be available in Cert:\CurrentUser\My.'
    }
    if ($certificate.Subject -eq $certificate.Issuer) {
        throw 'A public release needs a code signing certificate issued by a trusted CA.'
    }
    $signToolPath = (Get-Command $SignTool -CommandType Application -ErrorAction Stop).Source
    $signArgs = @('/sha1', $SigningCertificateThumbprint, '/fd', 'SHA256', '/tr', $TimestampUrl, '/td', 'SHA256')
}

$stage = New-Item -ItemType Directory -Path (Join-Path $project ('target\setup-' + [Guid]::NewGuid().ToString('N')))
try {
    $bridge = (New-Item -ItemType Directory -Path (Join-Path $stage.FullName 'bridge')).FullName
    $notices = (New-Item -ItemType Directory -Path (Join-Path $stage.FullName 'licenses')).FullName
    $fonts = (New-Item -ItemType Directory -Path (Join-Path $notices 'latin-modern')).FullName
    $output = (New-Item -ItemType Directory -Path (Join-Path $stage.FullName 'output')).FullName
    $stagedApp = Join-Path $stage.FullName 'literature-reader.exe'
    Copy-Item -LiteralPath $app.FullName -Destination $stagedApp
    if (!$SkipLayout) {
        # The desktop build ships the layout model and its runtime so figure and
        # table crops are refined offline. The Tern plugin downloads these later.
        $layoutSource = Join-Path $release 'runtime'
        foreach ($name in @('onnxruntime.dll', 'DirectML.dll', 'layout\doclayout.onnx')) {
            $source = Join-Path $layoutSource $name
            if (!(Test-Path -LiteralPath $source)) {
                throw "Layout runtime is missing $name under $layoutSource; stage it or pass -SkipLayout."
            }
            $destination = Join-Path (Join-Path $stage.FullName 'runtime') $name
            [void][IO.Directory]::CreateDirectory((Split-Path -Parent $destination))
            Copy-Item -LiteralPath $source -Destination $destination
        }
    }
    $compilerOptions = @('/Q', "/DAppVersion=$appVersion", "/DPayloadDir=$($stage.FullName)")
    if ($signing) {
        & $signToolPath sign @signArgs $stagedApp
        if ($LASTEXITCODE -ne 0) { throw 'Reader code signing failed.' }
        Assert-ReleaseSignature $stagedApp $SigningCertificateThumbprint

        # Inno expands $q and $f to quotes and the file it is signing.
        $signCommand = '$q' + $signToolPath.Replace('$', '$$') + '$q sign ' + ($signArgs -join ' ') + ' $f'
        $compilerOptions += @('/DReleaseSigning=1', "/SreaderRelease=$signCommand")
    }
    foreach ($file in @('bridge.mjs', 'package.json', 'package-lock.json')) {
        Copy-Item -LiteralPath (Join-Path $project ('bridge\' + $file)) -Destination $bridge
    }

    $cache = Join-Path $project 'target\setup-npm-cache'
    $npmOptions = @('ci', '--prefix', $bridge, '--cache', $cache, '--omit=dev', '--ignore-scripts', '--no-audit', '--no-fund', '--no-update-notifier')
    if ($Offline) { $npmOptions += '--offline' }
    & npm.cmd @npmOptions
    if ($LASTEXITCODE -ne 0) { throw 'Unable to install the locked translation dependencies.' }
    & (Join-Path $PSScriptRoot 'collect-bridge-checksums.ps1') -PayloadDir $stage.FullName

    Copy-Item -LiteralPath (Join-Path $project 'LICENSE') -Destination (Join-Path $notices 'literature-reader.txt')
    Copy-Item -LiteralPath (Join-Path $project 'src\data\SCOWL-Copyright.txt') -Destination $notices
    Get-ChildItem -LiteralPath (Join-Path $project 'src\data\latex-fonts') -File |
        Where-Object { $_.Extension -in '.txt', '.md' } |
        Copy-Item -Destination $fonts
    Copy-Item -LiteralPath (Join-Path (Split-Path -Parent $compilerPath) 'license.txt') -Destination (Join-Path $notices 'inno-setup.txt')
    & (Join-Path $PSScriptRoot 'collect-rust-notices.ps1') -OutputPath (Join-Path $notices 'rust-dependencies.txt')

    & $compilerPath @compilerOptions (Join-Path $PSScriptRoot 'literature-reader.iss')
    if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed.' }
    $installerName = "LiteratureReader-Setup-$appVersion-x64.exe"
    $stagedInstaller = Join-Path $output $installerName
    if ($signing) {
        Assert-ReleaseSignature $stagedInstaller $SigningCertificateThumbprint
    }
    [void][System.IO.Directory]::CreateDirectory($release)
    $destination = Join-Path $release $installerName
    Move-Item -LiteralPath $stagedInstaller -Destination $destination -Force
    Get-Item -LiteralPath $destination
} finally {
    $resolved = (Resolve-Path -LiteralPath $stage.FullName).ProviderPath
    $target = (Resolve-Path -LiteralPath (Join-Path $project 'target')).ProviderPath
    if ((Split-Path -Parent $resolved) -eq $target -and (Split-Path -Leaf $resolved) -match '^setup-[0-9a-f]{32}$' -and
        !((Get-Item -LiteralPath $resolved).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}
