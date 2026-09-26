[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$scriptPath = $PSCommandPath
if ([string]::IsNullOrWhiteSpace($scriptPath)) {
    throw 'could not resolve the M0 launcher acceptance script path'
}
$scriptDirectoryPath = [System.IO.Path]::GetDirectoryName($scriptPath)
if ([string]::IsNullOrWhiteSpace($scriptDirectoryPath)) {
    throw "could not resolve the directory for the M0 launcher acceptance script: $scriptPath"
}
$scriptDirectory = [System.IO.DirectoryInfo]::new($scriptDirectoryPath)
$testsDirectory = $scriptDirectory.Parent
$repositoryDirectory = if ($null -eq $testsDirectory) { $null } else { $testsDirectory.Parent }
if ($null -eq $repositoryDirectory) {
    throw "could not resolve the repository root from the M0 launcher acceptance script: $scriptPath"
}
$repositoryRoot = $repositoryDirectory.FullName
$launcher = Join-Path $repositoryRoot 'nagi.ps1'
if (-not (Test-Path -LiteralPath $launcher -PathType Leaf)) {
    throw "could not resolve the Nagi launcher from the M0 acceptance script: $launcher"
}

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
