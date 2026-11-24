$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'run-native.ps1') deletion-confirmation @args
exit $LASTEXITCODE
