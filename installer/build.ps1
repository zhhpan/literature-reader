#Requires -Version 5.1
[CmdletBinding()]
param(
    [string]$Compiler = 'ISCC.exe',
    [string]$AppExecutable,
    [switch]$Offline
)

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

$stage = New-Item -ItemType Directory -Path (Join-Path $project ('target\setup-' + [Guid]::NewGuid().ToString('N')))
try {
    $bridge = (New-Item -ItemType Directory -Path (Join-Path $stage.FullName 'bridge')).FullName
    $notices = (New-Item -ItemType Directory -Path (Join-Path $stage.FullName 'licenses')).FullName
    $fonts = (New-Item -ItemType Directory -Path (Join-Path $notices 'latin-modern')).FullName
    Copy-Item -LiteralPath $app.FullName -Destination (Join-Path $stage.FullName 'literature-reader.exe')
    foreach ($file in @('bridge.mjs', 'package.json', 'package-lock.json')) {
        Copy-Item -LiteralPath (Join-Path $project ('bridge\' + $file)) -Destination $bridge
    }

    $cache = Join-Path $project 'target\setup-npm-cache'
    $npmOptions = @('ci', '--prefix', $bridge, '--cache', $cache, '--omit=dev', '--ignore-scripts', '--no-audit', '--no-fund', '--no-update-notifier')
    if ($Offline) { $npmOptions += '--offline' }
    & npm.cmd @npmOptions
    if ($LASTEXITCODE -ne 0) { throw 'Unable to install the locked translation dependencies.' }

    Copy-Item -LiteralPath (Join-Path $project 'LICENSE') -Destination (Join-Path $notices 'literature-reader.txt')
    Copy-Item -LiteralPath (Join-Path $project 'src\data\SCOWL-Copyright.txt') -Destination $notices
    Get-ChildItem -LiteralPath (Join-Path $project 'src\data\latex-fonts') -File |
        Where-Object { $_.Extension -in '.txt', '.md' } |
        Copy-Item -Destination $fonts
    Copy-Item -LiteralPath (Join-Path (Split-Path -Parent $compilerPath) 'license.txt') -Destination (Join-Path $notices 'inno-setup.txt')
    & (Join-Path $PSScriptRoot 'collect-rust-notices.ps1') -OutputPath (Join-Path $notices 'rust-dependencies.txt')

    & $compilerPath /Q "/DAppVersion=$appVersion" "/DPayloadDir=$($stage.FullName)" (Join-Path $PSScriptRoot 'literature-reader.iss')
    if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed.' }
    Get-Item -LiteralPath (Join-Path $release "LiteratureReader-Setup-$appVersion-x64.exe")
} finally {
    $resolved = (Resolve-Path -LiteralPath $stage.FullName).ProviderPath
    $target = (Resolve-Path -LiteralPath (Join-Path $project 'target')).ProviderPath
    if ((Split-Path -Parent $resolved) -eq $target -and (Split-Path -Leaf $resolved) -match '^setup-[0-9a-f]{32}$' -and
        !((Get-Item -LiteralPath $resolved).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}
