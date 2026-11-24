param([string]$Subcommand)
$ErrorActionPreference = 'Stop'
$binary = Join-Path (Split-Path -Parent $PSScriptRoot) 'target/release/herdr-sbx.exe'
$action = $env:HERDR_PLUGIN_ACTION_ID
try {
    . (Join-Path $PSScriptRoot 'resolve-sbx.ps1')
    & (Join-Path $PSScriptRoot 'run-native.ps1') $subcommand
    exit $LASTEXITCODE
} catch {
    $message = "Cannot start the native action $action at $binary. Install the native component or build it with cargo build --release --locked. $_"
    $result = [ordered]@{ schemaVersion = 1; plugin = 'sbx.sandbox'; action = $action; ok = $false; errorKind = 'startup'; message = $message }
    Write-Output ('HERDR_SANDBOX_RESULT: ' + ($result | ConvertTo-Json -Compress))
    [Console]::Error.WriteLine($message)
    exit 1
}
