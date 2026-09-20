$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$launcher = Join-Path $repositoryRoot 'nagi.ps1'

& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcher clean
if ($LASTEXITCODE -ne 0) {
    throw "M7 clean failed with exit code $LASTEXITCODE"
}

$output = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcher run 2>&1)
$exitCode = $LASTEXITCODE
$output | ForEach-Object { Write-Output ([string] $_) }
if ($exitCode -ne 0) {
    throw "M7 QEMU launcher failed with exit code $exitCode"
}

$firstLog = Join-Path $repositoryRoot 'out\logs\m7-first-boot.log'
$finalLog = Join-Path $repositoryRoot 'out\logs\m1-qemu-boot.log'
$dataDisk = Join-Path $repositoryRoot 'out\artifacts\nagi-0.1-user-data.img'
foreach ($path in @($firstLog, $finalLog, $dataDisk)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "M7 required output was not created: $path"
    }
}
if ((Get-Item -LiteralPath $dataDisk).Length -ne 16MB) {
    throw "M7 persistent data disk has an unexpected size"
}

function Assert-OrderedMarkers([string] $path, [string[]] $markers, [string] $label) {
    $lines = [IO.File]::ReadAllLines($path)
    $lastLine = -1
    foreach ($marker in $markers) {
        $foundLine = -1
        for ($index = $lastLine + 1; $index -lt $lines.Length; $index++) {
            $matches = if ($marker -eq 'Hello from user space') {
                $lines[$index] -ceq $marker
            } else {
                $lines[$index].Contains($marker)
            }
            if ($matches) {
                $foundLine = $index
                break
            }
        }
        if ($foundLine -lt 0) {
            throw "$label does not contain the ordered marker: $marker"
        }
        $lastLine = $foundLine
    }
}

$common = @(
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
    'Nagi M7 storage START',
    'Nagi M7 VirtIO Block PASS',
    'Nagi M5 user process START',
    'Nagi M5 FPU state initial PASS',
    'Hello from user space',
    'Nagi M5 FPU state round-trip PASS',
    'Nagi M6 supervisor START',
    'Nagi M6 manifest dependency order PASS',
    'Nagi M6 service health PASS',
    'Nagi M6 service registry START',
    'Nagi M6 echo@1 call PASS'
)
Assert-OrderedMarkers $firstLog ($common + @(
    'Nagi M7 ext2 format PASS',
    'Nagi M7 file create PASS',
    'Nagi M7 file write PASS',
    'Nagi M7 file-backed mmap PASS',
    'Nagi M7 persistent write PASS'
)) 'M7 first-boot serial log'

$finalMarkers = $common + @(
    'Nagi M7 ext2 mount PASS',
    'Nagi M7 directory lookup PASS',
    'Nagi M7 file read PASS',
    'Nagi M7 file-backed mmap PASS',
    'Nagi M7 persistent read PASS',
    'Nagi M5 syscall PASS',
    'Nagi M5 acceptance PASS',
    'Nagi M6 acceptance PASS',
    'Nagi M7 acceptance PASS'
)
Assert-OrderedMarkers $finalLog $finalMarkers 'M7 final serial log'
Write-Output 'PASS M7 acceptance: ext2 file data survived a real QEMU reboot through the VirtIO Block disk'
