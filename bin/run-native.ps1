# Windows prevents Cargo from replacing an executable while a pane is running it.
# Keep one runnable copy per build, including for bridges launched asynchronously.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$build = Join-Path $root 'target/release/herdr-sbx.exe'
$share = [IO.FileShare]::Read -bor [IO.FileShare]::Delete
$source = [IO.File]::Open($build, [IO.FileMode]::Open, [IO.FileAccess]::Read, $share)
try {
    $sha = [Security.Cryptography.SHA256]::Create()
    try { $digest = [BitConverter]::ToString($sha.ComputeHash($source)).Replace('-', '') }
    finally { $sha.Dispose() }

    $dir = Join-Path (Join-Path $root 'target/herdr-runtime') $digest
    $exe = Join-Path $dir 'herdr-sbx.exe'
    if (-not [IO.File]::Exists($exe)) {
        [IO.Directory]::CreateDirectory($dir) | Out-Null
        $temp = Join-Path $dir ([Guid]::NewGuid().ToString('N') + '.tmp')
        try {
            $source.Position = 0
            $copy = [IO.File]::Open($temp, [IO.FileMode]::CreateNew)
            try { $source.CopyTo($copy) }
            finally { $copy.Dispose() }
            try { [IO.File]::Move($temp, $exe) }
            catch [IO.IOException] {
                # Another action may have published this same build first.
                if (-not [IO.File]::Exists($exe)) { throw }
            }
        } finally {
            [IO.File]::Delete($temp)
        }
    }
} finally {
    $source.Dispose()
}
& $exe @args
exit $LASTEXITCODE
