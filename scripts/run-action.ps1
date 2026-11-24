$ErrorActionPreference = 'Stop'
$binary = Join-Path $PSScriptRoot '../target/release/herdr-sbx.exe'
& $binary run-action @args
exit $LASTEXITCODE
