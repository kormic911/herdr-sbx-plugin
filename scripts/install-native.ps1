$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$manifest = Get-Content -Raw -LiteralPath (Join-Path $root 'herdr-plugin.toml')
if ($manifest -notmatch '(?m)^version = "(\d+\.\d+\.\d+)"\r?$') { throw 'Missing plugin version' }
$version = $Matches[1]
$arch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
if ($arch -ne 'AMD64') { throw "No native Windows release for $arch" }
$name = "herdr-sbx-v$version-x86_64-pc-windows-msvc.exe"
$url = "https://github.com/kormic911/herdr-sbx-plugin/releases/download/v$version/$name"
$dest = Join-Path $root 'target/release'
[IO.Directory]::CreateDirectory($dest) | Out-Null
$stage = Join-Path $dest ([Guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($stage) | Out-Null
try {
    $binary = Join-Path $stage $name
    Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $binary
    $checksum = Join-Path $stage 'checksum'
    Invoke-WebRequest -UseBasicParsing -Uri "$url.sha256" -OutFile $checksum
    $expected = [IO.File]::ReadAllText($checksum).Trim()
    $stream = [IO.File]::OpenRead($binary)
    $hash = [Security.Cryptography.SHA256]::Create()
    try { $actual = [BitConverter]::ToString($hash.ComputeHash($stream)).Replace('-', '') }
    finally { $hash.Dispose(); $stream.Dispose() }
    if ($expected -notmatch '^[0-9a-fA-F]{64}$' -or $actual -ine $expected) { throw 'Checksum mismatch' }
    Move-Item -Force -LiteralPath $binary -Destination (Join-Path $dest 'herdr-sbx.exe')
} finally {
    Remove-Item -LiteralPath $stage -Recurse -Force
}
