$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$launcher = Join-Path $repositoryRoot 'nagi.ps1'

& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcher clean
if ($LASTEXITCODE -ne 0) {
    throw "M14 clean failed with exit code $LASTEXITCODE"
}
$output = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcher m14 2>&1)
$exitCode = $LASTEXITCODE
$output | ForEach-Object { Write-Output ([string] $_) }
if ($exitCode -ne 0) {
    throw "M14 audio launcher failed with exit code $exitCode"
}
if (-not ($output | Where-Object { ([string] $_) -like 'PASS M14 audio: real VirtIO Sound playback, capture, mixer, volume/mute, and session gates passed*' })) {
    throw 'M14 audio acceptance marker was not emitted'
}
Write-Output 'PASS M14 audio acceptance wrapper'
