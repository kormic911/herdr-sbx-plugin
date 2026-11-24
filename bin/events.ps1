$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'resolve-sbx.ps1')
& (Join-Path $PSScriptRoot 'run-native.ps1') worktree-removed
exit $LASTEXITCODE
