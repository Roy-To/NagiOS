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
        throw "M12 clean failed with exit code $LASTEXITCODE"
    }

    $output = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcher network 2>&1)
    $exitCode = $LASTEXITCODE
    $output | ForEach-Object { Write-Output ([string] $_) }
    if ($exitCode -ne 0) {
        throw "M12 network launcher failed with exit code $exitCode"
    }
}
finally {
    if ($null -ne $server -and -not $server.HasExited) {
        Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
    }
}

$serialLog = Join-Path $repositoryRoot 'out\logs\m12-network.log'
if (-not (Test-Path -LiteralPath $serialLog -PathType Leaf)) {
    throw "M12 serial log was not created: $serialLog"
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
    'Nagi M12 network READY',
    'Nagi M12 DHCP PASS',
    'Nagi M12 ICMP PASS',
    'Nagi M12 UDP/DNS PASS',
    'Nagi M12 ARP PASS',
    'Nagi M12 TCP handshake PASS',
    'Nagi M12 HTTP response PASS',
    'Nagi M12 acceptance PASS'
)) {
    $foundLine = -1
    for ($index = $lastLine + 1; $index -lt $lines.Length; $index++) {
        if ($lines[$index].Contains($marker)) {
            $foundLine = $index
            break
        }
    }
    if ($foundLine -lt 0) {
        throw "M12 serial log does not contain ordered marker: $marker"
    }
    $lastLine = $foundLine
}
Write-Output 'PASS M12 acceptance: real QEMU guest performed DHCP, ICMP, UDP/DNS, ARP, TCP, and HTTP through its own nagi-net stack'
