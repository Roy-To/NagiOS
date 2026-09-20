$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$launcher = Join-Path $repositoryRoot 'nagi.ps1'

$output = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcher run 2>&1)
$exitCode = $LASTEXITCODE
$output | ForEach-Object { Write-Output ([string] $_) }
if ($exitCode -ne 0) {
    throw "M3 QEMU launcher failed with exit code $exitCode"
}

$serialLog = Join-Path $repositoryRoot 'out\logs\m1-qemu-boot.log'
if (-not (Test-Path -LiteralPath $serialLog -PathType Leaf)) {
    throw "M3 serial log was not created: $serialLog"
}
$serial = [IO.File]::ReadAllText($serialLog)
$requiredMarkers = @(
    'Nagi Kernel started',
    'Nagi M2 page allocation/free PASS',
    'Nagi M2 timer interrupts PASS',
    'Nagi Page fault handled (vector 14)',
    'Nagi invalid access diagnostic PASS',
    'Nagi M2 acceptance PASS',
    'Nagi M3 ACPI discovery PASS',
    'Nagi M3 SMP startup START',
    'Nagi M3 CPU 0 online/workload PASS',
    'Nagi M3 CPU 1 online/workload PASS',
    'Nagi M3 CPU 2 online/workload PASS',
    'Nagi M3 CPU 3 online/workload PASS',
    'Nagi M3 scheduler workloads PASS',
    'Nagi M3 acceptance PASS'
)
foreach ($marker in $requiredMarkers) {
    if (-not $serial.Contains($marker)) {
        throw "M3 serial log does not contain required marker: $marker"
    }
}
Write-Output 'PASS M3 acceptance: real QEMU guest brought four CPUs online and completed scheduler workloads'
