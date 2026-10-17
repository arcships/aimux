#!/usr/bin/env python3
"""Compile Java 8 bytecode and run JUnit with a supplied cached classpath.

Usage: mise exec java@temurin-17 gradle@8 flutter@3 -- python3 verify-local.py /path/to/classpath.txt
The classpath must contain JNA 5.14, Jackson 2.17, JUnit Jupiter/Platform, AssertJ and org.json.
This fallback avoids Gradle's lock-listener socket when local sandbox networking is unavailable.
"""
from pathlib import Path
import os
import platform
import shutil
import subprocess
import sys

root = Path(__file__).resolve().parent
classpath = Path(sys.argv[1]).read_text().strip()
out = root / 'build' / 'verify'
for directory in ('main', 'test', 'native'):
    shutil.rmtree(out / directory, ignore_errors=True)
    (out / directory).mkdir(parents=True, exist_ok=True)
fixture = out / 'native' / ('libaimux_abi_fixture.dylib' if platform.system() == 'Darwin' else 'libaimux_abi_fixture.so')
shared = '-dynamiclib' if platform.system() == 'Darwin' else '-shared'
commands = [
    ['cc', '-std=c11', '-D_POSIX_C_SOURCE=200809L', '-Wall', '-Wextra', '-Werror', shared,
     '-fPIC', '-I' + str(root / '../../aimux-ffi/include'), str(root / 'src/test/native/abi_fixture.c'), '-o', str(fixture)],
    ['javac', '-J-XX:ActiveProcessorCount=2', '--release', '8', '-encoding', 'UTF-8', '-cp', classpath,
     '-d', str(out / 'main')] + [str(p) for p in sorted(list((root / 'src/main/java').rglob('*.java')) + list((root / 'generated').rglob('*.java')))],
    ['javac', '-J-XX:ActiveProcessorCount=2', '--release', '8', '-encoding', 'UTF-8', '-cp',
     str(out / 'main') + os.pathsep + classpath, '-d', str(out / 'test')]
     + [str(p) for p in sorted((root / 'src/test/java').rglob('*.java'))],
    ['java', '-XX:ActiveProcessorCount=2', '-Daimux.testLibrary=' + str(fixture),
     '-Djna.library.path=' + os.environ.get('AIMUX_FFI_LIB_DIR', str(root / '../../target/debug')), '-cp',
     os.pathsep.join((str(out / 'main'), str(out / 'test'), classpath)),
     'ai.arcships.aimux.ContractTestRunner'],
]
for command in commands:
    print('Running:', command[0], flush=True)
    subprocess.run(command, cwd=root, check=True)
