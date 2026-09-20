$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$launcher = Join-Path $repositoryRoot 'nagi.ps1'

& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcher clean
if ($LASTEXITCODE -ne 0) {
    throw "M16 clean failed with exit code $LASTEXITCODE"
}
$output = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcher m16 2>&1)
$exitCode = $LASTEXITCODE
$output | ForEach-Object { Write-Output ([string] $_) }
if ($exitCode -ne 0) {
    throw "M16 package/SDK launcher failed with exit code $exitCode"
}
if (-not ($output | Where-Object { ([string] $_) -like 'PASS M16 package/SDK:*' })) {
    throw 'M16 package/SDK acceptance marker was not emitted'
}
Write-Output 'PASS M16 package SDK acceptance wrapper'
