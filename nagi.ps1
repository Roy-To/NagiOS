[CmdletBinding()]
param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]] $Arguments
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$cargoCommand = Get-Command cargo -ErrorAction SilentlyContinue
$cargoPath = $null
if ($null -ne $cargoCommand) {
    $cargoPath = $cargoCommand.Source
}
$rustupCommand = Get-Command rustup -ErrorAction SilentlyContinue
if ($null -ne $rustupCommand) {
    $toolchainText = Get-Content -LiteralPath (Join-Path $repositoryRoot 'rust-toolchain.toml') -Raw
    $toolchainMatch = [regex]::Match($toolchainText, '(?m)^\s*channel\s*=\s*"([^"]+)"')
    if ($toolchainMatch.Success) {
        $toolchain = $toolchainMatch.Groups[1].Value
        $pinnedCargo = & $rustupCommand.Source which cargo --toolchain $toolchain 2>$null
        $cargoStatus = $LASTEXITCODE
        $pinnedRustc = & $rustupCommand.Source which rustc --toolchain $toolchain 2>$null
        $rustcStatus = $LASTEXITCODE
        $pinnedRustdoc = & $rustupCommand.Source which rustdoc --toolchain $toolchain 2>$null
        $rustdocStatus = $LASTEXITCODE
        if ($cargoStatus -eq 0 -and $rustcStatus -eq 0 -and $rustdocStatus -eq 0 -and
            -not [string]::IsNullOrWhiteSpace([string] $pinnedCargo) -and
            -not [string]::IsNullOrWhiteSpace([string] $pinnedRustc) -and
            -not [string]::IsNullOrWhiteSpace([string] $pinnedRustdoc)) {
            $cargoPath = [string] $pinnedCargo
            $env:RUSTC = [string] $pinnedRustc
            $env:RUSTDOC = [string] $pinnedRustdoc
        }
    }
}
if ([string]::IsNullOrWhiteSpace($cargoPath)) {
    $userCargoPath = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
    if (Test-Path -LiteralPath $userCargoPath) {
        $cargoPath = $userCargoPath
    }
}
if ([string]::IsNullOrWhiteSpace($cargoPath)) {
    Write-Error 'Cargo was not found. Install the Rust toolchain from rust-toolchain.toml.'
    exit 4
}

$fetchMode = $Arguments.Count -gt 0 -and $Arguments[0] -eq 'fetch'
$manifestPath = if ($fetchMode) {
    Join-Path $repositoryRoot 'tools\nagi-bootstrap\Cargo.toml'
} else {
    Join-Path $repositoryRoot 'Cargo.toml'
}
$packageName = if ($fetchMode) { 'nagi-bootstrap' } else { 'nagi-cli' }
Push-Location -LiteralPath $repositoryRoot
try {
    $llvmLinker = Join-Path ${env:ProgramFiles} 'LLVM\bin\lld-link.exe'
    if (-not (Test-Path -LiteralPath $llvmLinker)) {
        $lldCommand = Get-Command lld-link.exe -ErrorAction SilentlyContinue
        if ($null -ne $lldCommand) {
            $llvmLinker = $lldCommand.Source
        }
    }
    if (Test-Path -LiteralPath $llvmLinker) {
        $env:RUSTC_LINKER = $llvmLinker
        $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = $llvmLinker
        $msvcRoot = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\2022\BuildTools\VC\Tools\MSVC'
        $msvc = Get-ChildItem -LiteralPath $msvcRoot -Directory -ErrorAction SilentlyContinue |
            Sort-Object Name -Descending | Select-Object -First 1
        $windowsKitRoot = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\Lib'
        $windowsKit = Get-ChildItem -LiteralPath $windowsKitRoot -Directory -ErrorAction SilentlyContinue |
            Sort-Object Name -Descending | Select-Object -First 1
        if ($null -ne $msvc -and $null -ne $windowsKit) {
            $env:LIB = @(
                (Join-Path $msvc.FullName 'lib\x64'),
                (Join-Path $windowsKit.FullName 'um\x64'),
                (Join-Path $windowsKit.FullName 'ucrt\x64')
            ) -join ';'
        }
    }
    $ErrorActionPreference = 'Continue'
    $cargoOutput = @(& $cargoPath run --locked --manifest-path $manifestPath -p $packageName -- @Arguments 2>&1)
    $cargoOutput | ForEach-Object { Write-Output ([string] $_) }
    $childExitCode = $LASTEXITCODE
}
finally {
    $ErrorActionPreference = 'Stop'
    Pop-Location
}
exit $childExitCode
