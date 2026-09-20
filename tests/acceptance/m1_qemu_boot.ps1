$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$launcher = Join-Path $repositoryRoot 'nagi.ps1'

$output = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcher run 2>&1)
$exitCode = $LASTEXITCODE
$output | ForEach-Object { Write-Output ([string] $_) }
if ($exitCode -ne 0) {
    throw "M1 QEMU launcher failed with exit code $exitCode"
}

$serialLog = Join-Path $repositoryRoot 'out\logs\m1-qemu-boot.log'
if (-not (Test-Path -LiteralPath $serialLog -PathType Leaf)) {
    throw "M1 serial log was not created: $serialLog"
}
$serial = [IO.File]::ReadAllText($serialLog)
if (-not $serial.Contains('Nagi Kernel started')) {
    throw "M1 serial log does not contain the kernel acceptance line"
}
Write-Output "PASS M1 acceptance: QEMU serial log contains Nagi Kernel started"
