#!/usr/bin/env python3
"""Compile the DTO vector runner with cached Jackson jars, without Gradle."""
from pathlib import Path
import os
import shutil
import subprocess
import sys

root = Path(__file__).resolve().parents[1]
cache = Path.home() / '.gradle/caches/modules-2/files-2.1/com.fasterxml.jackson.core'
jars = []
for artifact in ('jackson-annotations', 'jackson-core', 'jackson-databind'):
    matches = sorted((cache / artifact / '2.17.2').glob('*/*.jar'))
    if not matches:
        raise SystemExit(f'Cached Jackson 2.17.2 is missing: {artifact}')
    jars.append(str(matches[0]))
java_home = os.environ.get('JAVA_HOME')
if not java_home and sys.platform == 'darwin' and shutil.which('mise'):
    result = subprocess.run(['mise', 'where', 'java@temurin-17'], text=True, capture_output=True)
    if result.returncode == 0:
        java_home = result.stdout.strip()

def tool(name):
    return str(Path(java_home) / 'bin' / name) if java_home else name

out = root / 'build/dto-vectors'
out.mkdir(parents=True, exist_ok=True)
classpath = os.pathsep.join(jars)
sources = sorted((root / 'generated/ai/arcships/aimux/types').glob('*.java'))
sources.append(root / 'src/test/java/ai/arcships/aimux/DtoVectors.java')
subprocess.run([tool('javac'), '-J-XX:ActiveProcessorCount=2', '--release', '8', '-encoding', 'UTF-8',
                '-cp', classpath, '-d', str(out)] + [str(p) for p in sources], check=True)
subprocess.run([tool('java'), '-XX:ActiveProcessorCount=2', '-cp', str(out) + os.pathsep + classpath,
                'ai.arcships.aimux.DtoVectors', str(root / '../../contract-tests/dto')], check=True)
