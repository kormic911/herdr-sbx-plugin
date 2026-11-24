$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'resolve-sbx.ps1')
& (Join-Path $PSScriptRoot 'run-native.ps1') sandboxes @args
exit $LASTEXITCODE
