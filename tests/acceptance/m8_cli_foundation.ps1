$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$launcher = Join-Path $repositoryRoot 'nagi.ps1'

& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcher clean
if ($LASTEXITCODE -ne 0) {
    throw "M8 clean failed with exit code $LASTEXITCODE"
}

$commands = @(
    'pwd',
    'ls',
    'cat nagi-persistent.txt',
    'nagi ps',
    'nagi mem',
    'nagi log',
    'exit'
) -join "`n"
$output = @($commands | & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcher shell 2>&1)
$exitCode = $LASTEXITCODE
$output | ForEach-Object { Write-Output ([string] $_) }
if ($exitCode -ne 0) {
    throw "M8 shell launcher failed with exit code $exitCode"
}

$serialLog = Join-Path $repositoryRoot 'out\logs\m8-shell.log'
if (-not (Test-Path -LiteralPath $serialLog -PathType Leaf)) {
    throw "M8 serial log was not created: $serialLog"
}
$lines = [IO.File]::ReadAllLines($serialLog)
function Assert-OrderedMarkers([string[]] $markers) {
    $lastLine = -1
    foreach ($marker in $markers) {
        $foundLine = -1
        for ($index = $lastLine + 1; $index -lt $lines.Length; $index++) {
            $matches = if ($marker -eq 'Hello from user space') {
                $lines[$index] -ceq $marker
            } else {
                $lines[$index].Contains($marker)
            }
            if ($matches) {
                $foundLine = $index
                break
            }
        }
        if ($foundLine -lt 0) {
            throw "M8 serial log does not contain the ordered marker: $marker"
        }
        $lastLine = $foundLine
    }
}

Assert-OrderedMarkers @(
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
    'Nagi M8 nsh START',
    'Nagi M8 pwd PASS',
    'Nagi M8 ls PASS',
    'Nagi M8 cat PASS',
    'Nagi M8 ps PASS',
    'Nagi M8 mem PASS',
    'Nagi M8 log PASS',
    'Nagi M8 acceptance PASS'
)
if (-not ($lines -contains 'Nagi OS persistent storage')) {
    throw 'M8 cat did not print the real persistent guest file payload'
}
if (-not (($lines -join "`n").Contains('nagi-persistent.txt'))) {
    throw 'M8 ls did not print the real persistent guest directory entry'
}
Write-Output 'PASS M8 acceptance: real Nagi nsh inspected guest files and process/memory/log state'

