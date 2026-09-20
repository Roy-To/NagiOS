$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$launcher = Join-Path $repositoryRoot 'nagi.ps1'

& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcher clean
if ($LASTEXITCODE -ne 0) {
    throw "M13 clean failed with exit code $LASTEXITCODE"
}
$output = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcher m13 2>&1)
$exitCode = $LASTEXITCODE
$output | ForEach-Object { Write-Output ([string] $_) }
if ($exitCode -ne 0) {
    throw "M13 unified launcher failed with exit code $exitCode"
}
if (-not ($output -contains 'PASS M13 unified acceptance: real QEMU POSIX/relibc and Rust std gates passed')) {
    throw 'M13 unified acceptance marker was not emitted'
}
Write-Output 'PASS M13 unified acceptance wrapper'
