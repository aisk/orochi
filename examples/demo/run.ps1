# Builds the demo extension module and imports it with a real Python.
#
#   .\examples\demo\run.ps1 [-Python <path to python.exe>]
#
# Without -Python, the interpreter is found with the `py` launcher (`py -3`)
# or as `python` on PATH. The DLL is copied to target/pyd/demo.pyd (the name
# must match the module name, `demo`, for Python to find `PyInit_demo`).
param(
    [string]$Python = ""
)
$ErrorActionPreference = "Stop"
$root = Resolve-Path "$PSScriptRoot\..\.."

if (-not $Python) {
    if (Get-Command py -ErrorAction SilentlyContinue) {
        $Python = (& py -3 -c "import sys; print(sys.executable)").Trim()
    } elseif (Get-Command python -ErrorAction SilentlyContinue) {
        $Python = (Get-Command python).Source
    } else {
        throw "Python not found; pass -Python <path to python.exe>"
    }
}

# pyo3 builds against the interpreter it finds on PATH (or `PYO3_PYTHON`
# if set); it must be the same Python version as `$Python`.
cargo build -p demo --manifest-path "$root\Cargo.toml"
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$out = Join-Path $root "target\pyd"
New-Item -ItemType Directory -Force $out | Out-Null
Copy-Item (Join-Path $root "target\debug\demo.dll") (Join-Path $out "demo.pyd") -Force

$env:PYTHONPATH = $out
& $Python (Join-Path $PSScriptRoot "check.py")
exit $LASTEXITCODE
