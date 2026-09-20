$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$launcher = Join-Path $repositoryRoot 'nagi.ps1'

& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcher clean
if ($LASTEXITCODE -ne 0) {
    throw "M10 clean failed with exit code $LASTEXITCODE"
}

$output = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcher desktop 2>&1)
$exitCode = $LASTEXITCODE
$output | ForEach-Object { Write-Output ([string] $_) }
if ($exitCode -ne 0) {
    throw "M10 desktop launcher failed with exit code $exitCode"
}

$serialLog = Join-Path $repositoryRoot 'out\logs\m10-desktop.log'
if (-not (Test-Path -LiteralPath $serialLog -PathType Leaf)) {
    throw "M10 serial log was not created: $serialLog"
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
    'Nagi boot stage PLATFORM 15',
    'Nagi M6 echo@1 call PASS',
    'Nagi boot stage CORE_SERVICES 30',
    'Nagi M7 ext2 mount PASS',
    'Nagi M7 persistent read PASS',
    'Nagi boot stage STORAGE 50',
    'Nagi M5 syscall PASS',
    'Nagi M6 acceptance PASS',
    'Nagi M7 acceptance PASS',
    'Nagi boot stage GRAPHICS 70',
    'Nagi boot stage SESSION 90',
    'Nagi boot lock READY',
    'Nagi boot collapse COMPLETE',
    'Nagi boot frame checksum=',
    'Nagi boot lock checksum=',
    'Nagi M10 desktop READY',
    'Nagi M10 surface checksum=',
    'Nagi M10 Calculator focus PASS',
    'Nagi M10 Notes focus PASS',
    'Nagi M10 Japanese input PASS',
    'Nagi M10 Files focus PASS',
    'Nagi M10 GUI Terminal focus PASS',
    'Nagi M10 acceptance PASS'
)) {
    $foundLine = -1
    for ($index = $lastLine + 1; $index -lt $lines.Length; $index++) {
        if ($lines[$index].Contains($marker)) {
            $foundLine = $index
            break
        }
    }
    if ($foundLine -lt 0) {
        throw "M10 serial log does not contain ordered marker: $marker"
    }
    $lastLine = $foundLine
}
Write-Output 'PASS M10 acceptance: real QEMU input focused all Nagi desktop windows and entered Japanese text'
