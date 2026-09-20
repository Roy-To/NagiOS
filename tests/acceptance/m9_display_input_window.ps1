$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$launcher = Join-Path $repositoryRoot 'nagi.ps1'

& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcher clean
if ($LASTEXITCODE -ne 0) {
    throw "M9 clean failed with exit code $LASTEXITCODE"
}

$output = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcher gui 2>&1)
$exitCode = $LASTEXITCODE
$output | ForEach-Object { Write-Output ([string] $_) }
if ($exitCode -ne 0) {
    throw "M9 GUI launcher failed with exit code $exitCode"
}

$serialLog = Join-Path $repositoryRoot 'out\logs\m9-gui.log'
if (-not (Test-Path -LiteralPath $serialLog -PathType Leaf)) {
    throw "M9 serial log was not created: $serialLog"
}
$lines = [IO.File]::ReadAllLines($serialLog)
$lastLine = -1
foreach ($marker in @(
    'Nagi Kernel started',
    'Nagi M2 acceptance PASS',
    'Nagi M3 acceptance PASS',
    'Nagi M4 acceptance PASS',
    'Nagi M7 VirtIO Block PASS',
    'Nagi M5 user process START',
    'Nagi M6 echo@1 call PASS',
    'Nagi M7 ext2 mount PASS',
    'Nagi M7 persistent read PASS',
    'Nagi M5 syscall PASS',
    'Nagi M6 acceptance PASS',
    'Nagi M7 acceptance PASS',
    'Nagi M9 window READY',
    'Nagi M9 mouse move PASS',
    'Nagi M9 state x=',
    'Nagi M9 focus PASS',
    'Nagi M9 keyboard PASS',
    'Nagi M9 acceptance PASS'
)) {
    $foundLine = -1
    for ($index = $lastLine + 1; $index -lt $lines.Length; $index++) {
        if ($lines[$index].Contains($marker)) {
            $foundLine = $index
            break
        }
    }
    if ($foundLine -lt 0) {
        throw "M9 serial log does not contain ordered marker: $marker"
    }
    $lastLine = $foundLine
}
Write-Output 'PASS M9 acceptance: real QEMU VirtIO mouse and keyboard input moved and focused a Nagi window'
