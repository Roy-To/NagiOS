[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\\..')).Path
$launcher = Join-Path $repositoryRoot 'nagi.ps1'

$null = @(& $launcher doctor --unexpected 2>&1)
if ($LASTEXITCODE -ne 2) {
    throw "launcher did not preserve usage exit code 2 (got $LASTEXITCODE)"
}

$null = @(& $launcher image 2>&1)
if ($LASTEXITCODE -ne 0) {
    throw "launcher did not preserve image success exit code 0 (got $LASTEXITCODE)"
}

Write-Output 'PASS M0 Windows launcher exit propagation'
