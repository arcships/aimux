"""Run Python tests against an already-built production PyO3 cdylib.

Usage: python3 tests/run_real.py /absolute/path/to/libaimux.dylib
Set AIMUX_HTTP_TESTS=1 to include independent localhost provider fixtures.
This development loader does not replace wheel installation verification.
"""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

binding = Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix='aimux-python-real-') as folder:
    package = Path(folder) / 'aimux'
    shutil.copytree(binding / 'python/aimux', package, ignore=shutil.ignore_patterns('__pycache__', '*.so'))
    filename = 'aimux.pyd' if sys.platform == 'win32' else 'aimux.abi3.so'
    shutil.copyfile(sys.argv[1], package / filename)
    env = dict(os.environ, PYTHONPATH=folder, AIMUX_REAL_OPS='1')
    env.pop('AIMUX_OPS_DOUBLE', None)
    subprocess.run([sys.executable, '-m', 'unittest', 'discover', '-s', str(binding / 'tests'),
                    '-p', 'test_ops_*.py', '-v', *sys.argv[2:]], env=env, check=True, timeout=120)
