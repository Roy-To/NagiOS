$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$launcher = Join-Path $repositoryRoot 'nagi.ps1'

$output = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcher run 2>&1)
$exitCode = $LASTEXITCODE
$output | ForEach-Object { Write-Output ([string] $_) }
if ($exitCode -ne 0) {
    throw "M4 QEMU launcher failed with exit code $exitCode"
}

$serialLog = Join-Path $repositoryRoot 'out\logs\m1-qemu-boot.log'
if (-not (Test-Path -LiteralPath $serialLog -PathType Leaf)) {
    throw "M4 serial log was not created: $serialLog"
}
$serial = [IO.File]::ReadAllText($serialLog)
$requiredMarkers = @(
    'Nagi Kernel started',
    'Nagi M2 acceptance PASS',
    'Nagi M3 acceptance PASS',
    'Nagi M4 handles/VMO/IPC START',
    'Nagi M4 VMO basics PASS',
    'Nagi M4 channel round-trip PASS',
    'Nagi M4 rights attenuation PASS',
    'Nagi M4 wait primitives PASS',
    'Nagi M4 acceptance PASS'
)
foreach ($marker in $requiredMarkers) {
    if (-not $serial.Contains($marker)) {
        throw "M4 serial log does not contain required marker: $marker"
    }
}
Write-Output 'PASS M4 acceptance: real guest exercised VMO, Channel transfer, rights attenuation, and wait primitives'
