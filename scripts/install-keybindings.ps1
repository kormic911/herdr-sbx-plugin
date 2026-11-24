$ErrorActionPreference = 'Stop'
$binary = Join-Path $PSScriptRoot '../target/release/herdr-sbx.exe'
& $binary install-keybindings
exit $LASTEXITCODE
