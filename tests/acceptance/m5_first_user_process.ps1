$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$launcher = Join-Path $repositoryRoot 'nagi.ps1'

$output = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcher run 2>&1)
$exitCode = $LASTEXITCODE
$output | ForEach-Object { Write-Output ([string] $_) }
if ($exitCode -ne 0) {
    throw "M5 QEMU launcher failed with exit code $exitCode"
}

$serialLog = Join-Path $repositoryRoot 'out\logs\m1-qemu-boot.log'
if (-not (Test-Path -LiteralPath $serialLog -PathType Leaf)) {
    throw "M5 serial log was not created: $serialLog"
}
$lines = [IO.File]::ReadAllLines($serialLog)
$requiredMarkers = @(
    'Nagi Kernel started',
    'Nagi M2 page allocation/free PASS',
    'Nagi M3 ACPI discovery PASS',
    'Nagi M2 timer interrupts PASS',
    'Nagi Page fault handled (vector 14)',
    'Nagi invalid access diagnostic PASS',
    'Nagi M2 acceptance PASS',
    'Nagi Page fault resume PASS',
    'Nagi M3 SMP startup START',
    'Nagi M3 CPU 0 online/workload PASS',
    'Nagi M3 CPU 1 online/workload PASS',
    'Nagi M3 CPU 2 online/workload PASS',
    'Nagi M3 CPU 3 online/workload PASS',
    'Nagi M3 scheduler workloads PASS',
    'Nagi M3 acceptance PASS',
    'Nagi M4 handles/VMO/IPC START',
    'Nagi M4 VMO basics PASS',
    'Nagi M4 channel round-trip PASS',
    'Nagi M4 rights attenuation PASS',
    'Nagi M4 wait primitives PASS',
    'Nagi M4 acceptance PASS',
    'Nagi M5 user process START',
    'Nagi M5 FPU state initial PASS',
    'Hello from user space',
    'Nagi M5 FPU state round-trip PASS',
    'Nagi M5 syscall PASS',
    'Nagi M5 acceptance PASS'
)
$lastLine = -1
foreach ($marker in $requiredMarkers) {
    $foundLine = -1
    for ($index = $lastLine + 1; $index -lt $lines.Length; $index++) {
        $line = $lines[$index]
        $matches = if ($marker -eq 'Hello from user space') {
            $line -ceq $marker
        } else {
            $line.Contains($marker)
        }
        if ($matches) {
            $foundLine = $index
            break
        }
    }
    if ($foundLine -lt 0) {
        throw "M5 serial log does not contain the ordered marker: $marker"
    }
    $lastLine = $foundLine
}
Write-Output 'PASS M5 acceptance: real user INIT.ELF printed through Nagi SYSCALL and exited successfully'
