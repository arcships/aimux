"""Build an isolated native adapter with a dispatcher double, then run tests.

The production manifest is never rewritten. Set AIMUX_BUILD_COMMAND to override
the build executable. This suite verifies the PyO3 boundary independently of
provider behavior.
"""
import os
from pathlib import Path
import shutil
import signal
import shlex
import subprocess
import sys
import tempfile

binding = Path(__file__).resolve().parents[1]
repo = binding.parents[1]
with tempfile.TemporaryDirectory(prefix="aimux-python-contract-") as folder:
    root = Path(folder)
    copied = root / "binding"
    shutil.copytree(binding, copied, ignore=shutil.ignore_patterns("target", "__pycache__", "Cargo.lock", "*.so"))
    double = root / "ops-double"
    shutil.copytree(binding / "tests/ops-double", double)
    core = repo / 'aimux-core'
    if os.environ.get('AIMUX_CORE_DOUBLE') == '1':
        core = root / 'core-double'
        shutil.copytree(binding / 'tests/core-double', core)
        shutil.copyfile(repo / 'aimux-core/src/abort_signal.rs', core / 'src/abort_signal.rs')
        print('BOUNDARY MODE: core error double + real AbortSignal source; real core is unverified.', flush=True)
    manifest = copied / "Cargo.toml"
    text = manifest.read_text().replace('../../aimux-core', str(core))
    text = text.replace('../../aimux-ops', str(double))
    manifest.write_text(text)
    double_manifest = double / "Cargo.toml"
    double_manifest.write_text(double_manifest.read_text().replace('../../../../aimux-core', str(core)))
    pin = copied / "generated/ops.py"
    pin.write_text(pin.read_text().replace(
        next(line for line in pin.read_text().splitlines() if line.startswith("MANIFEST_DIGEST = ")),
        'MANIFEST_DIGEST = "isolated-test-double"',
    ))
    env = dict(os.environ, PYO3_PYTHON=sys.executable)
    command = shlex.split(env.get('AIMUX_BUILD_COMMAND', 'cargo')) + ['build', '--manifest-path', str(manifest), '--message-format=json']
    if env.get('AIMUX_CONTRACT_TARGET'):
        command.extend(['--target-dir', env['AIMUX_CONTRACT_TARGET']])
    process = subprocess.Popen(command, env=env, text=True, stdout=subprocess.PIPE, start_new_session=True)
    try:
        output, _ = process.communicate(timeout=1200)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGTERM)
        process.communicate()
        raise
    for line in output.splitlines():
        if line.startswith('{'):
            item = __import__('json').loads(line)
            if item.get('reason') == 'compiler-message':
                print(item['message'].get('rendered', ''), end='')
    if process.returncode:
        sys.exit(process.returncode)
    import json
    artifacts = [json.loads(line) for line in output.splitlines() if line.startswith('{')]
    libraries = [path for item in artifacts if item.get('reason') == 'compiler-artifact' and item['target']['name'] == 'aimux'
                 for path in item['filenames'] if path.endswith(('.dylib', '.so', '.dll'))]
    if len(libraries) != 1:
        raise RuntimeError('expected exactly one native library, got %r' % libraries)
    package = copied / 'python/aimux'
    shutil.copyfile(libraries[0], package / 'aimux.abi3.so')
    env['PYTHONPATH'] = str(copied / 'python')
    env['AIMUX_OPS_DOUBLE'] = '1'
    env.pop('AIMUX_REAL_OPS', None)
    env.pop('AIMUX_HTTP_TESTS', None)
    subprocess.run([sys.executable, '-m', 'unittest', 'discover', '-s', str(binding / 'tests'), '-p', 'test_ops_*.py', '-v'],
                   env=env, check=True)
