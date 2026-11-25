#Requires -Version 5.1
[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$PayloadDir
)

$ErrorActionPreference = 'Stop'
$payload = (Resolve-Path -LiteralPath $PayloadDir).ProviderPath
$bridge = Join-Path $payload 'bridge'
$sdk = (Resolve-Path -LiteralPath (Join-Path $bridge 'node_modules')).ProviderPath
if (!(Test-Path -LiteralPath (Join-Path $sdk '@tencent-ai\agent-sdk\package.json') -PathType Leaf)) {
    throw 'The staged translation SDK is missing.'
}

$encoding = [Text.UTF8Encoding]::new($false)
$launcherLines = foreach ($name in @('bridge.mjs', 'package.json', 'package-lock.json')) {
    $hash = (Get-FileHash -LiteralPath (Join-Path $bridge $name) -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash $name"
}
[IO.File]::WriteAllLines((Join-Path $payload 'reader-bridge-launcher.sha256'), [string[]]$launcherLines, $encoding)

$lines = foreach ($file in (Get-ChildItem -LiteralPath $sdk -File -Recurse -Force | Sort-Object FullName)) {
    $relative = $file.FullName.Substring($sdk.Length + 1)
    # These are the same exclusions as the installer entry for node_modules.
    if ($relative -eq '.package-lock.json' -or
        $relative -match '^@tencent-ai\\agent-sdk\\cli\\vendor\\(?:ripgrep\\[^\\]+-(?:darwin|linux)\\|genie-trash\\(?:darwin|linux)-)') {
        continue
    }
    if ($file.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw "The staged SDK contains a linked file: $relative"
    }
    $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash $relative"
}
[IO.File]::WriteAllLines((Join-Path $payload 'reader-bridge-sdk.sha256'), [string[]]$lines, $encoding)
