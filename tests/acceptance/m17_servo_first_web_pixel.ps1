[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Push-Location -LiteralPath $repositoryRoot
try {
    $output = & .\nagi.ps1 m17 2>&1
    $exitCode = $LASTEXITCODE
    $output | ForEach-Object { Write-Output ([string] $_) }
    if ($exitCode -ne 0) { exit $exitCode }
    if (-not ($output -join "`n" | Select-String -SimpleMatch 'PASS M17 first web pixel:')) {
        throw 'M17 CLI did not report the real first-web-pixel pass marker.'
    }
    $serialLog = Join-Path $repositoryRoot 'out\logs\m17-servo.log'
    if (-not (Test-Path -LiteralPath $serialLog)) { throw "M17 serial log was not created: $serialLog" }
    $serial = Get-Content -LiteralPath $serialLog -Raw
    if ($serial -notmatch 'Nagi M17 first web pixel checksum=0x') { throw 'M17 checksum marker missing.' }
    if ($serial -notmatch 'Nagi M17 first web pixel PASS') { throw 'M17 guest pass marker missing.' }
    Write-Output 'PASS M17 first web pixel acceptance: real Servo guest frame reached Nagi Surface and QEMU'
}
finally {
    Pop-Location
}
