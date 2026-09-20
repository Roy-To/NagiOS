$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$launcher = Join-Path $repositoryRoot 'nagi.ps1'

& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcher clean
if ($LASTEXITCODE -ne 0) {
    throw "M15 clean failed with exit code $LASTEXITCODE"
}
$output = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcher m15 2>&1)
$exitCode = $LASTEXITCODE
$output | ForEach-Object { Write-Output ([string] $_) }
if ($exitCode -ne 0) {
    throw "M15 history launcher failed with exit code $exitCode"
}
if (-not ($output | Where-Object { ([string] $_) -like 'PASS M15 history: real guest create/edit/move/delete/restore/undo and persistent ledger passed*' })) {
    throw 'M15 history acceptance marker was not emitted'
}
Write-Output 'PASS M15 history acceptance wrapper'
