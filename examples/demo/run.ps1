# Builds the demo extension module and imports it with a real Python.
#
#   .\examples\demo\run.ps1 [-Python <path to python.exe>] [-TypeCheck]
#
# Without -Python, the interpreter is found with the `py` launcher (`py -3`)
# or as `python` on PATH. The DLL is copied to target/pyd/demo.pyd (the name
# must match the module name, `demo`, for Python to find `PyInit_demo`), and
# check.py writes the generated type stubs to target/pyd/stubs.
#
# -TypeCheck also checks the stubs with mypy (fetched by `uv`, which must be
# installed; nothing is installed into the repository): `stubtest` against the
# built module, typecheck/good.py must pass, and typecheck/bad.py must fail
# with exactly the errors marked `# expect: <code>`. Then the same samples
# with pyright in strict mode (fetched by `uvx`; it downloads Node.js on
# first use).
param(
    [string]$Python = "",
    [switch]$TypeCheck
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

$stubs = Join-Path $out "stubs"
if (Test-Path $stubs) { Remove-Item -Recurse -Force $stubs }
$env:PYTHONPATH = $out
& $Python (Join-Path $PSScriptRoot "check.py") $stubs
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
if (-not $TypeCheck) { exit 0 }

if (-not (Get-Command uv -ErrorAction SilentlyContinue)) {
    throw "-TypeCheck needs uv (https://docs.astral.sh/uv/)"
}
$env:MYPYPATH = $stubs
$env:MYPY_CACHE_DIR = Join-Path $root "target\mypy_cache"
# `--directory`: stubtest keeps its cache in the current directory.
$mypy = @("--directory", $out, "run", "--no-project", "--python", $Python, "--with", "mypy==2.4.1", "python", "-m")
$samples = Join-Path $PSScriptRoot "typecheck"

Write-Host "stubtest demo:"
& uv @mypy mypy.stubtest demo
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "mypy --strict typecheck/good.py:"
& uv @mypy mypy --strict (Join-Path $samples "good.py")
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "mypy --strict typecheck/bad.py (must fail as marked):"
$bad = Join-Path $samples "bad.py"
$report = & uv @mypy mypy --strict $bad
if ($LASTEXITCODE -eq 0) { throw "typecheck/bad.py passed the type check" }
$found = @($report | ForEach-Object {
    if ($_ -match 'bad\.py:(\d+): error: .*\[([a-z-]+)\]$') { "$($Matches[1]): $($Matches[2])" }
})
$expected = @(Select-String -Path $bad -Pattern '# expect: ([a-z-]+)' | ForEach-Object {
    "$($_.LineNumber): $($_.Matches[0].Groups[1].Value)"
})
$diff = Compare-Object $expected $found
if ($diff) {
    $report | Write-Host
    $diff | Format-Table | Out-String | Write-Host
    throw "typecheck/bad.py: errors differ from the `# expect:` markers (<= expected only, => found only)"
}
Write-Host "bad.py: all $($expected.Count) expected errors reported, no others"

# The same with pyright in strict mode (its rule names differ from mypy's
# error codes, so for bad.py only the lines are compared).
$config = Join-Path $out "pyrightconfig.json"
@{ typeCheckingMode = "strict"; stubPath = $stubs; pythonVersion = "3.10"; reportMissingModuleSource = "none" } |
    ConvertTo-Json | Set-Content -Encoding utf8 $config
$pyright = @("--directory", $out, "--python", $Python, "pyright@1.1.407", "--project", $config)

Write-Host "pyright typecheck/good.py:"
& uvx @pyright (Join-Path $samples "good.py")
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "pyright typecheck/bad.py (must fail on the marked lines):"
$report = & uvx @pyright $bad
if ($LASTEXITCODE -eq 0) { throw "typecheck/bad.py passed pyright" }
$found = @($report | ForEach-Object {
    if ($_ -match 'bad\.py:(\d+):\d+ - error:') { $Matches[1] }
} | Sort-Object -Unique { [int]$_ })
$expected = @($expected | ForEach-Object { ($_ -split ':')[0] } | Sort-Object -Unique { [int]$_ })
$diff = Compare-Object $expected $found
if ($diff) {
    $report | Write-Host
    $diff | Format-Table | Out-String | Write-Host
    throw "typecheck/bad.py: pyright errors are not on exactly the marked lines (<= expected only, => found only)"
}
Write-Host "bad.py: pyright reports errors on all $($expected.Count) marked lines, no others"
exit 0
