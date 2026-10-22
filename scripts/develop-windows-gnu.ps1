# Build rustmesh into a Python environment on Windows with rustup's GNU
# toolchain (no Visual Studio needed).
#
#   .\scripts\develop-windows-gnu.ps1 -Python C:\path\to\env\python.exe
#
# Without -Python, the python.exe found on PATH is used. Release wheels are
# built with MSVC; this script is for local work.
param(
    [string]$Python = "python",
    [switch]$Debug
)
# maturin reports progress on stderr, which Windows PowerShell 5.1 turns into
# error records; failure is judged by the exit code instead.
$ErrorActionPreference = "Continue"
$Python = (Get-Command $Python -ErrorAction Stop).Source
$envRoot = Split-Path -Parent $Python
$repo = Split-Path -Parent $PSScriptRoot

$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
# Keep the build output out of the source folder, which may be synchronised.
if (-not $env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = "$env:USERPROFILE\.cargo-target\rustmesh" }
# maturin installs into the environment named here.
$env:CONDA_PREFIX = $envRoot
# PyO3 links Python through raw-dylib by default, which on windows-gnu needs a
# full MinGW binutils (dlltool + as). Link the environment's python3.lib instead.
$env:PYO3_USE_RAW_DYLIB = "0"
$env:RUSTFLAGS = "-L native=$envRoot\libs"

$buildArgs = @("develop")
if (-not $Debug) { $buildArgs += "--release" }
Push-Location $repo
try {
    & $Python -m maturin @buildArgs
    if ($LASTEXITCODE -ne 0) { throw "maturin develop failed ($LASTEXITCODE)" }
} finally {
    Pop-Location
}
