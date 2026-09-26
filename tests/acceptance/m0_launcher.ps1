[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$scriptPath = $MyInvocation.MyCommand.Path
if ([string]::IsNullOrWhiteSpace($scriptPath)) {
    throw 'could not resolve the M0 launcher acceptance script path'
}
$scriptDirectory = Split-Path -Parent $scriptPath
$repositoryRoot = (Resolve-Path (Join-Path $scriptDirectory '../..')).Path
$launcher = Join-Path $repositoryRoot 'nagi.ps1'

$invalidOutput = @(& $launcher doctor --unexpected 2>&1)
if ($LASTEXITCODE -ne 2) {
    $invalidOutput | ForEach-Object { Write-Output ([string] $_) }
    throw "launcher did not preserve usage exit code 2 (got $LASTEXITCODE)"
}

$helpOutput = @(& $launcher --help 2>&1)
if ($LASTEXITCODE -ne 0) {
    $helpOutput | ForEach-Object { Write-Output ([string] $_) }
    throw "launcher did not preserve help success exit code 0 (got $LASTEXITCODE)"
}
if (($helpOutput -join "`n") -notmatch 'Nagi OS developer orchestrator') {
    $helpOutput | ForEach-Object { Write-Output ([string] $_) }
    throw 'launcher help output was not recognized'
}

Write-Output 'PASS M0 Windows launcher exit propagation'
