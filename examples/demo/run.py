"""Builds the demo extension module and imports it with this Python.

    python examples/demo/run.py

The library is built against the running interpreter and copied to
`target/pyd/` as `demo.pyd` (Windows) or `demo.so` (Linux/macOS); the name
must match the module name, `demo`, for Python to find `PyInit_demo`.
"""

import os
import shutil
import subprocess
import sys
from pathlib import Path

here = Path(__file__).resolve().parent
root = here.parent.parent

env = dict(os.environ, PYO3_PYTHON=sys.executable)
if sys.platform != "win32":
    env["PYO3_BUILD_EXTENSION_MODULE"] = "1"
cargo = ["cargo", "build", "-p", "demo", "--manifest-path", str(root / "Cargo.toml")]
if build := subprocess.run(cargo, env=env).returncode:
    sys.exit(build)

if sys.platform == "win32":
    built, name = "demo.dll", "demo.pyd"
elif sys.platform == "darwin":
    built, name = "libdemo.dylib", "demo.so"
else:
    built, name = "libdemo.so", "demo.so"
out = root / "target" / "pyd"
out.mkdir(parents=True, exist_ok=True)
shutil.copyfile(root / "target" / "debug" / built, out / name)

env = dict(os.environ, PYTHONPATH=str(out))
sys.exit(subprocess.run([sys.executable, str(here / "check.py")], env=env).returncode)
