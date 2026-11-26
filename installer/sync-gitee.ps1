#Requires -Version 5.1
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Tag,
    [Parameter(Mandatory)][string]$AssetsDir,
    [Parameter(Mandatory)][string]$Token,
    [string]$Repository = 'zhhpan/literature-reader',
    [string]$GiteeRepository = 'zhhpan/literature-reader',
    [string]$GitHubToken,
    [switch]$NoPush
)

# Mirrors a GitHub release to Gitee. Gitee caps a single release attachment at
# 100 MB, so the installer is split into parts and gitee-update.json lists them
# for the updater to download, concatenate and verify.
$ErrorActionPreference = 'Stop'
$version = $Tag.TrimStart('v')
$api = "https://gitee.com/api/v5/repos/$GiteeRepository"
$installer = Get-ChildItem -LiteralPath $AssetsDir -Filter "LiteratureReader-Setup-$version-x64.exe" |
    Select-Object -First 1
if (!$installer) { throw "Installer for $Tag is missing from $AssetsDir." }
$digest = (Get-FileHash -LiteralPath $installer.FullName -Algorithm SHA256).Hash.ToLowerInvariant()

# 60 MB parts keep each attachment comfortably below Gitee's 100 MB limit.
$partSize = 60MB
$bytes = [IO.File]::ReadAllBytes($installer.FullName)
$parts = @()
$index = 1
for ($offset = 0; $offset -lt $bytes.Length; $offset += $partSize) {
    $length = [Math]::Min($partSize, $bytes.Length - $offset)
    $name = "$($installer.Name).part$index"
    $path = Join-Path $AssetsDir $name
    $slice = New-Object byte[] $length
    [Array]::Copy($bytes, $offset, $slice, 0, $length)
    [IO.File]::WriteAllBytes($path, $slice)
    $parts += [pscustomobject]@{ Name = $name; Path = $path; Size = $length }
    $index += 1
}

function Invoke-Gitee([string]$Method, [string]$Uri, $Body) {
    if ($null -eq $Body) { return Invoke-RestMethod -Method $Method -Uri $Uri }
    Invoke-RestMethod -Method $Method -Uri $Uri -Body $Body
}

function Upload-Attachment([int]$ReleaseId, [string]$Path) {
    $json = & curl.exe -sS -X POST "$api/releases/$ReleaseId/attach_files" `
        -F "access_token=$Token" -F "file=@$Path"
    if ($LASTEXITCODE -ne 0) { throw "Gitee attachment upload failed for $(Split-Path -Leaf $Path)." }
    $attachment = $json | ConvertFrom-Json
    if (!$attachment.browser_download_url) {
        throw "Gitee did not return a download URL for $(Split-Path -Leaf $Path): $json"
    }
    $attachment
}

$giteeUrl = "https://oauth2:$Token@gitee.com/$GiteeRepository.git"
# Make the tag and the release commit available on Gitee before the API call.
& git -C (Split-Path -Parent $PSScriptRoot) push --force $giteeUrl "HEAD:refs/heads/master" "refs/tags/${Tag}:refs/tags/${Tag}"
if ($LASTEXITCODE -ne 0) { throw 'Could not push the tag to Gitee.' }

$releases = Invoke-Gitee GET "$api/releases?access_token=$Token"
$existing = @($releases) | Where-Object tag_name -eq $Tag | Select-Object -First 1
if ($existing) {
    Invoke-Gitee DELETE "$api/releases/$($existing.id)?access_token=$Token" | Out-Null
}
$release = Invoke-Gitee POST "$api/releases" @{
    access_token = $Token
    tag_name = $Tag
    name = $Tag
    body = "Gitee mirror of https://github.com/$Repository/releases/tag/$Tag"
    prerelease = 'false'
    target_commitish = $Tag
}

# Everything except the oversized installer; each existing asset stays below
# the attachment cap and is mirrored for completeness.
$other = Get-ChildItem -LiteralPath $AssetsDir -File | Where-Object {
    $_.FullName -ne $installer.FullName -and $_.Name -notlike '*.part*'
}
$uploaded = @{}
foreach ($part in $parts) {
    $attachment = Upload-Attachment $release.id $part.Path
    $uploaded[$part.Name] = $attachment
}
foreach ($file in $other) {
    $attachment = Upload-Attachment $release.id $file.FullName
    $uploaded[$file.Name] = $attachment
}

$manifest = [ordered]@{
    tag_name = $Tag
    draft = $false
    prerelease = $false
    assets = @([ordered]@{
        name = $installer.Name
        state = 'uploaded'
        size = $bytes.Length
        browser_download_url = "https://gitee.com/$GiteeRepository/releases/download/$Tag/$($installer.Name)"
        digest = "sha256:$digest"
        parts = @($parts | ForEach-Object {
            [ordered]@{
                name = $_.Name
                size = $_.Size
                url = $uploaded[$_.Name].browser_download_url
            }
        })
    })
}
$root = Split-Path -Parent $PSScriptRoot
$manifestPath = Join-Path $root 'gitee-update.json'
[IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 8 -Compress))
Write-Output "wrote $manifestPath"

if ($NoPush) { return }
$message = "Update the Gitee manifest for $Tag"
& git -C $root add gitee-update.json
& git -C $root -c user.name='github-actions[bot]' -c user.email='41898282+github-actions[bot]@users.noreply.github.com' commit -m $message
if ($LASTEXITCODE -ne 0) { throw 'Could not commit gitee-update.json.' }
& git -C $root push $giteeUrl "HEAD:refs/heads/master"
if ($LASTEXITCODE -ne 0) { throw 'Could not push the Gitee manifest.' }
if ($GitHubToken) {
    $githubUrl = "https://x-access-token:$GitHubToken@github.com/$Repository.git"
    & git -C $root push $githubUrl "HEAD:refs/heads/main"
    if ($LASTEXITCODE -ne 0) { throw 'Could not push the manifest to GitHub.' }
}
