#Requires -Version 5.1
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$AppExecutable,
    [Parameter(Mandatory)][string]$NodeExe,
    [Parameter(Mandatory)][string]$PopplerDir,
    [Parameter(Mandatory)][string]$BridgeDir,
    [string]$Version = '',
    [Parameter(Mandatory)][string]$OutputDir,
    [string]$ReleaseTag,
    [string]$Repository = 'zhhpan/literature-reader'
)

# The Tern plugin's portable backend: only the files the reader actually
# executes. The Poppler list is the recursive import closure of the five
# tools for the pinned poppler-windows build (26.09.0); update it with the
# dependency when the pinned Poppler version changes.
$ErrorActionPreference = 'Stop'
$popplerFiles = @(
    'pdfimages.exe', 'pdfinfo.exe', 'pdftohtml.exe', 'pdftoppm.exe', 'pdftotext.exe',
    'poppler.dll', 'Lerc.dll', 'MSVCP140.dll', 'VCRUNTIME140.dll', 'VCRUNTIME140_1.dll',
    'vcruntime140.dll', 'deflate.dll', 'freetype.dll', 'icudt78.dll', 'icuuc78.dll',
    'jpeg8.dll', 'lcms2.dll', 'libcrypto-3-x64.dll', 'libcurl.dll', 'liblzma.dll',
    'libpng16.dll', 'libssh2.dll', 'openjp2.dll', 'psl-5.dll', 'tiff.dll', 'zlib.dll',
    'zstd.dll'
)

$app = Get-Item -LiteralPath $AppExecutable
$node = Get-Item -LiteralPath $NodeExe
$poppler = if (Test-Path -LiteralPath (Join-Path $PopplerDir 'Library\bin')) {
    Join-Path $PopplerDir 'Library\bin'
} else {
    $PopplerDir
}
$bridge = Get-Item -LiteralPath $BridgeDir
$versionLine = Select-String -LiteralPath (Join-Path (Split-Path -Parent $PSScriptRoot) 'Cargo.toml') -Pattern '^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"\s*$'
if ($versionLine) { $Version = $versionLine.Matches[0].Groups[1].Value }

$stage = Join-Path ([IO.Path]::GetTempPath()) ('tern-backend-' + [Guid]::NewGuid().ToString('N'))
$output = New-Item -ItemType Directory -Path $OutputDir -Force
try {
    New-Item -ItemType Directory -Path (Join-Path $stage 'runtime\poppler\Library\bin') -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $stage 'runtime\node') -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $stage 'bridge') -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $stage 'licenses') -Force | Out-Null

    Copy-Item -LiteralPath $app.FullName -Destination (Join-Path $stage 'literature-reader.exe')
    Copy-Item -LiteralPath $node.FullName -Destination (Join-Path $stage 'runtime\node\node.exe')
    foreach ($name in $popplerFiles) {
        $source = Join-Path $poppler $name
        if (!(Test-Path -LiteralPath $source)) { throw "Poppler runtime is missing $name." }
        Copy-Item -LiteralPath $source -Destination (Join-Path $stage ('runtime\poppler\Library\bin\' + $name))
    }
    foreach ($name in @('bridge.mjs', 'package.json')) {
        Copy-Item -LiteralPath (Join-Path $bridge.FullName $name) -Destination (Join-Path $stage 'bridge')
    }
    Copy-Item -LiteralPath (Join-Path $bridge.FullName 'node_modules') -Destination (Join-Path $stage 'bridge\node_modules') -Recurse

    # Remove what the bridge never loads: other platforms' tools, the browser
    # UI, the lite bundle the headless transport does not use, and tests.
    $sdk = Join-Path $stage 'bridge\node_modules\@tencent-ai\agent-sdk'
    $drop = @(
        'cli\vendor\ripgrep\arm64-darwin', 'cli\vendor\ripgrep\arm64-linux',
        'cli\vendor\ripgrep\x64-darwin', 'cli\vendor\ripgrep\x64-linux',
        'cli\vendor\genie-trash\darwin-arm64', 'cli\vendor\genie-trash\darwin-x64',
        'cli\vendor\genie-trash\linux-arm64', 'cli\vendor\genie-trash\linux-x64',
        'cli\vendor\sandbox-center-runtime-kit',
        'cli\dist\web-ui', 'cli\dist\codebuddy-lite-wb.mjs', 'cli\dist\lazy-lite-wb',
        'cli\bin\native\win32-arm64'
    )
    foreach ($relative in $drop) {
        $target = Join-Path $sdk $relative
        if (Test-Path -LiteralPath $target) { Remove-Item -LiteralPath $target -Recurse -Force }
    }
    Get-ChildItem -LiteralPath (Join-Path $stage 'bridge\node_modules') -Recurse -File |
        Where-Object { $_.Name -match '\.(map|test\.[cm]?[jt]s|spec\.[cm]?[jt]s)$' } |
        Remove-Item -Force

    Copy-Item -LiteralPath (Join-Path (Split-Path -Parent $PSScriptRoot) 'LICENSE') -Destination (Join-Path $stage 'licenses\literature-reader.txt')
    $nodeLicense = Join-Path (Split-Path -Parent $node.FullName) 'LICENSE'
    if (Test-Path -LiteralPath $nodeLicense) { Copy-Item -LiteralPath $nodeLicense -Destination (Join-Path $stage 'licenses\node.txt') }
    $sdkLicense = Join-Path $sdk 'cli\LICENSE.txt'
    if (Test-Path -LiteralPath $sdkLicense) { Copy-Item -LiteralPath $sdkLicense -Destination (Join-Path $stage 'licenses\agent-sdk.txt') }

    $zipName = 'literature-reader-tern-backend-win-x64.zip'
    $zip = Join-Path $output $zipName
    Remove-Item -LiteralPath $zip -Force -ErrorAction SilentlyContinue
    & tar.exe -a -c -f $zip -C $stage .
    if ($LASTEXITCODE -ne 0) { throw 'Unable to write the Tern backend zip.' }

    $hash = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
    $url = if ($ReleaseTag) {
        "https://github.com/$Repository/releases/download/$ReleaseTag/$zipName"
    } else {
        ''
    }
    $manifest = [ordered]@{
        schema = 1
        version = $Version
        platform = 'win32-x64'
        url = $url
        sha256 = $hash
        bytes = (Get-Item -LiteralPath $zip).Length
    } | ConvertTo-Json -Compress
    [IO.File]::WriteAllText((Join-Path $output 'literature-reader-tern.json'), $manifest)
    Get-Item -LiteralPath $zip, (Join-Path $output 'literature-reader-tern.json')
} finally {
    if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
}
