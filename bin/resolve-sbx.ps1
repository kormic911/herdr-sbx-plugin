# A server started before an MSI install still has its original process PATH.
# config.sbxBin takes precedence over this child-process environment override.
if ($env:OS -eq 'Windows_NT' -and [string]::IsNullOrEmpty($env:HERDR_SBX_BIN)) {
    $command = Get-Command sbx.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($command) {
        $env:HERDR_SBX_BIN = $command.Source
    } else {
        $directories = @(
            foreach ($scope in 'Machine', 'User') {
                [Environment]::GetEnvironmentVariable('Path', $scope) -split ';'
            }
            foreach ($base in $env:LOCALAPPDATA, $env:ProgramFiles) {
                if ($base) { Join-Path $base 'DockerSandboxes/bin' }
            }
        )
        foreach ($dir in $directories) {
            if ([string]::IsNullOrWhiteSpace($dir)) { continue }
            $dir = [Environment]::ExpandEnvironmentVariables($dir.Trim().Trim('"'))
            if (-not [IO.Path]::IsPathRooted($dir)) { continue }
            $candidate = [IO.Path]::Combine($dir, 'sbx.exe')
            if ([IO.File]::Exists($candidate)) {
                $env:HERDR_SBX_BIN = $candidate
                break
            }
        }
    }
}
