$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$launcher = Join-Path $repositoryRoot 'nagi.ps1'
$fixtureRoot = Join-Path $repositoryRoot 'tests\fixtures\m12'
$python = Get-Command python.exe -ErrorAction Stop
$server = $null
try {
    $server = Start-Process -FilePath $python.Source -ArgumentList @(
        '-m', 'http.server', '18080', '--bind', '0.0.0.0', '--directory', $fixtureRoot
    ) -PassThru -WindowStyle Hidden
    Start-Sleep -Milliseconds 500

    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcher clean
    if ($LASTEXITCODE -ne 0) {
        throw "M13 clean failed with exit code $LASTEXITCODE"
    }
    $output = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcher posix 2>&1)
    $exitCode = $LASTEXITCODE
    $output | ForEach-Object { Write-Output ([string] $_) }
    if ($exitCode -ne 0) {
        throw "M13 posix launcher failed with exit code $exitCode"
    }
}
finally {
    if ($null -ne $server -and -not $server.HasExited) {
        Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
    }
}

$serialLog = Join-Path $repositoryRoot 'out\logs\m13-posix.log'
if (-not (Test-Path -LiteralPath $serialLog -PathType Leaf)) {
    throw "M13 serial log was not created: $serialLog"
}
$lines = [IO.File]::ReadAllLines($serialLog)
$lastLine = -1
foreach ($marker in @(
    'Nagi Kernel started',
    'Nagi M2 acceptance PASS',
    'Nagi M3 acceptance PASS',
    'Nagi M4 acceptance PASS',
    'Nagi M7 VirtIO Block PASS',
    'Nagi M12 VirtIO Net PASS',
    'Nagi M5 user process START',
    'Nagi M6 echo@1 call PASS',
    'Nagi M7 ext2 mount PASS',
    'Nagi M7 persistent read PASS',
    'Nagi M5 syscall PASS',
    'Nagi M6 acceptance PASS',
    'Nagi M7 acceptance PASS',
    'Nagi M13 Rust PAL PASS',
    'Nagi M13 C POSIX PASS',
    'Nagi M13 socket/DNS PASS',
    'Nagi M13 mmap PASS',
    'Nagi M13 time/sleep PASS',
    'Nagi M13 poll PASS',
    'Nagi M13 thread/TLS PASS',
    'Nagi M13 native spawn PASS',
    'Nagi M13 relibc C PASS',
    'Nagi M13 OSS library PASS',
    'Nagi M13 acceptance PASS'
)) {
    $foundLine = -1
    for ($index = $lastLine + 1; $index -lt $lines.Length; $index++) {
        if ($lines[$index].Contains($marker)) {
            $foundLine = $index
            break
        }
    }
    if ($foundLine -lt 0) {
        throw "M13 serial log does not contain ordered marker: $marker"
    }
    $lastLine = $foundLine
}
Write-Output 'PASS M13 acceptance: real QEMU guest exercised Nagi PAL, C POSIX, and relibc over guest VFS/network'
