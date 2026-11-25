#Requires -Version 5.1
[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$OutputPath,
    [string]$Target = 'x86_64-pc-windows-msvc'
)

$ErrorActionPreference = 'Stop'
$manifest = Join-Path (Split-Path -Parent $PSScriptRoot) 'Cargo.toml'
$metadataJson = & cargo metadata --manifest-path $manifest --locked --filter-platform $Target --format-version 1
if ($LASTEXITCODE -ne 0) {
    throw 'Unable to read the locked Rust dependency graph.'
}
$metadata = $metadataJson | ConvertFrom-Json

$notices = [System.Text.StringBuilder]::new()
[void]$notices.AppendLine('Third-party Rust dependencies for Literature Reader')
[void]$notices.AppendLine("Target $Target")
[void]$notices.AppendLine()

$dependencies = $metadata.packages |
    Where-Object { $_.id -ne $metadata.resolve.root } |
    Sort-Object name, version
foreach ($package in $dependencies) {
    $directory = Split-Path -Parent $package.manifest_path
    $files = @(Get-ChildItem -LiteralPath $directory -File |
        Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE|COPYRIGHT)([._-]|$)' })
    if ($package.license_file) {
        $files += Get-Item -LiteralPath (Join-Path $directory $package.license_file)
    }
    $files = @($files | Sort-Object FullName -Unique)
    if ($files.Count -eq 0) {
        throw "No license or copyright text was found for $($package.name) $($package.version)."
    }

    [void]$notices.AppendLine(('=' * 72))
    [void]$notices.AppendLine("$($package.name) $($package.version)")
    if ($package.license) {
        [void]$notices.AppendLine($package.license)
    }
    foreach ($file in $files) {
        [void]$notices.AppendLine()
        [void]$notices.AppendLine($file.Name)
        [void]$notices.AppendLine([System.IO.File]::ReadAllText($file.FullName).TrimEnd())
    }
    [void]$notices.AppendLine()
}

$sysroot = & rustc --print sysroot
if ($LASTEXITCODE -ne 0) {
    throw 'Unable to locate the Rust toolchain notices.'
}
$rustDocs = Join-Path $sysroot 'share/doc/rust'
$libraryNotice = Join-Path $rustDocs 'COPYRIGHT-library.html'
if (!(Test-Path -LiteralPath $libraryNotice -PathType Leaf)) {
    $libraryNotice = Join-Path $rustDocs 'COPYRIGHT.html'
}
if (!(Test-Path -LiteralPath $libraryNotice -PathType Leaf)) {
    throw 'The Rust toolchain does not include its bundled copyright notice.'
}

$destination = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputPath)
$outputDirectory = Split-Path -Parent $destination
[void][System.IO.Directory]::CreateDirectory($outputDirectory)
[System.IO.File]::WriteAllText($destination, $notices.ToString())
Copy-Item -LiteralPath $libraryNotice -Destination (Join-Path $outputDirectory 'rust-standard-library.html')
