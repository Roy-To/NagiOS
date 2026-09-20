$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$launcher = Join-Path $repositoryRoot 'nagi.ps1'

& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcher clean
if ($LASTEXITCODE -ne 0) {
    throw "M11 clean failed with exit code $LASTEXITCODE"
}

$output = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcher security 2>&1)
$exitCode = $LASTEXITCODE
$output | ForEach-Object { Write-Output ([string] $_) }
if ($exitCode -ne 0) {
    throw "M11 security launcher failed with exit code $exitCode"
}

$serialLog = Join-Path $repositoryRoot 'out\logs\m11-security.log'
if (-not (Test-Path -LiteralPath $serialLog -PathType Leaf)) {
    throw "M11 serial log was not created: $serialLog"
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
    'Nagi M11 local login PASS',
    'Nagi M11 lock screen PASS',
    'Nagi M11 Developer Mode PASS',
    'Nagi M11 trusted dialog ASK PASS',
    'Nagi M11 malicious file DENIED',
    'Nagi M11 malicious microphone DENIED',
    'Nagi M11 acceptance PASS'
)) {
    $foundLine = -1
    for ($index = $lastLine + 1; $index -lt $lines.Length; $index++) {
        if ($lines[$index].Contains($marker)) {
            $foundLine = $index
            break
        }
    }
    if ($foundLine -lt 0) {
        throw "M11 serial log does not contain ordered marker: $marker"
    }
    $lastLine = $foundLine
}
Write-Output 'PASS M11 acceptance: real QEMU guest enforced login, lock screen, and Permission Broker denial'
