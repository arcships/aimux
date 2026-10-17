#!/usr/bin/env python3
"""Build a deterministic new-ABI test double, never a substitute for Rust QA."""
import pathlib
import subprocess
import sys

root = pathlib.Path(__file__).resolve().parents[1]
output = root / '.dart_tool/native'
output.mkdir(parents=True, exist_ok=True)
suffix = '.dylib' if sys.platform == 'darwin' else '.so'
shared = '-dynamiclib' if sys.platform == 'darwin' else '-shared'
for name, defines in [('ops_fixture', []), ('bad_abi', ['-DBAD_ABI']),
                      ('bad_manifest', ['-DBAD_MANIFEST']),
                      ('missing_symbol', ['-DOMIT_ERROR_MESSAGE']),
                      ('missing_finalizer', ['-DOMIT_HANDLE_FINALIZER'])]:
    command = ['cc', '-std=c11', '-Wall', '-Wextra', '-Werror', '-fPIC', shared,
               '-pthread', *defines, str(root / 'native/aimux_dart.c'),
               str(root / 'test/native/ops_fixture.c'), '-o', str(output / (name + suffix))]
    subprocess.run(command, check=True)
    print(output / (name + suffix))
