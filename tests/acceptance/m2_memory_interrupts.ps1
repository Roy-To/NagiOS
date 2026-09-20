$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$launcher = Join-Path $repositoryRoot 'nagi.ps1'

$output = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcher run 2>&1)
$exitCode = $LASTEXITCODE
$output | ForEach-Object { Write-Output ([string] $_) }
if ($exitCode -ne 0) {
    throw "M2 QEMU launcher failed with exit code $exitCode"
}

$serialLog = Join-Path $repositoryRoot 'out\logs\m1-qemu-boot.log'
if (-not (Test-Path -LiteralPath $serialLog -PathType Leaf)) {
    throw "M2 serial log was not created: $serialLog"
}
$serial = [IO.File]::ReadAllText($serialLog)
$requiredMarkers = @(
    'Nagi Kernel started',
    'Nagi M2 page allocation/free PASS',
    'Nagi M2 timer interrupts PASS',
    'Nagi Page fault handled (vector 14)',
    'Nagi invalid access diagnostic PASS',
    'Nagi M2 acceptance PASS'
)
foreach ($marker in $requiredMarkers) {
    if (-not $serial.Contains($marker)) {
        throw "M2 serial log does not contain required marker: $marker"
    }
}
Write-Output 'PASS M2 acceptance: real QEMU guest passed memory, timer, and page-fault checks'
