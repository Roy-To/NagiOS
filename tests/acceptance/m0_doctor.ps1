[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$output = @(& (Join-Path $repositoryRoot 'nagi.ps1') doctor 2>&1)
$exitCode = $LASTEXITCODE
$output | ForEach-Object { Write-Output $_ }
if ($exitCode -ne 0) {
    throw "M0 doctor acceptance failed with exit code $exitCode"
}

$dependencies = @(
    'Git', 'Rust (rustc)', 'Cargo', 'Rustup', 'LLVM/Clang', 'LLD',
    'QEMU', 'OVMF CODE/VARS', 'CMake', 'Meson', 'Ninja', 'Python'
)
foreach ($dependency in $dependencies) {
    if (-not ($output | Where-Object { $_ -match "^PASS $([regex]::Escape($dependency)):\s" })) {
        throw "M0 doctor did not report PASS for $dependency"
    }
}
if (-not ($output | Where-Object { $_ -match '^PASS doctor:' })) {
    throw 'M0 doctor summary was not reported as PASS'
}
Write-Output 'PASS M0 Windows doctor acceptance'

