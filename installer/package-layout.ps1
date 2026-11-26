#Requires -Version 5.1
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$AppExecutable,
    [Parameter(Mandatory)][string]$RuntimeDir,
    [Parameter(Mandatory)][string]$OutputDir,
    [string]$Version = '0.1.10',
    [string]$ReleaseTag,
    [string]$Repository = 'zhhpan/literature-reader'
)

# Optional Tern enhancement: layout model + ONNX Runtime + DirectML, plus the
# layout-capable backend executable (older plugin cores are upgraded in place).
$ErrorActionPreference = 'Stop'
$app = Get-Item -LiteralPath $AppExecutable
$runtime = Get-Item -LiteralPath $RuntimeDir
$files = @(
    @{ Source = $app.FullName; Target = 'literature-reader-layout.exe' },
    @{ Source = Join-Path $runtime.FullName 'onnxruntime.dll'; Target = 'runtime/onnxruntime.dll' },
    @{ Source = Join-Path $runtime.FullName 'DirectML.dll'; Target = 'runtime/DirectML.dll' },
    @{ Source = Join-Path $runtime.FullName 'layout/doclayout.onnx'; Target = 'runtime/layout/doclayout.onnx' }
)
$stage = Join-Path ([IO.Path]::GetTempPath()) ('tern-layout-' + [Guid]::NewGuid().ToString('N'))
$output = New-Item -ItemType Directory -Path $OutputDir -Force
try {
    foreach ($file in $files) {
        if (!(Test-Path -LiteralPath $file.Source)) { throw "Layout asset is missing: $($file.Source)" }
        $target = Join-Path $stage ($file.Target -replace '/', '\')
        $directory = Split-Path -Parent $target
        [void][IO.Directory]::CreateDirectory($directory)
        Copy-Item -LiteralPath $file.Source -Destination $target
    }
    $licenses = Join-Path $stage 'licenses'
    [void][IO.Directory]::CreateDirectory($licenses)
    Copy-Item -LiteralPath (Join-Path (Split-Path -Parent $PSScriptRoot) 'LICENSE') -Destination (Join-Path $licenses 'literature-reader.txt')

    $zipName = 'literature-reader-tern-layout-win-x64.zip'
    $zip = Join-Path $output $zipName
    Remove-Item -LiteralPath $zip -Force -ErrorAction SilentlyContinue
    & tar.exe -a -c -f $zip -C $stage .
    if ($LASTEXITCODE -ne 0) { throw 'Unable to write the layout zip.' }
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
    [IO.File]::WriteAllText((Join-Path $output 'literature-reader-tern-layout.json'), $manifest)
    Get-Item -LiteralPath $zip, (Join-Path $output 'literature-reader-tern-layout.json')
} finally {
    if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
}
