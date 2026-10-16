"""Replay every shared vector through the production PyO3 call boundary.

Only a scripted source is injected at the real Dispatcher's public test seam.
This suite runs in ordinary pytest/unittest discovery, including without the
optional provider integration environment flags. Release wheels have no seam.
"""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


class SharedNativeVectors(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        binding = Path(__file__).resolve().parents[1]
        root = Path(os.environ.get('AIMUX_CONTRACT_VECTORS', binding.parents[1] / 'contract-tests/ops'))
        cls.paths = sorted((root if root.name == 'scenarios' else root / 'scenarios').glob('*.json'))
        if len(cls.paths) != 11:
            raise AssertionError('all 11 shared stream scenarios must be replayed')
        native_library = os.environ.get('AIMUX_PYTHON_CONTRACT_LIBRARY')
        if not native_library:
            command = [os.environ.get('AIMUX_BUILD_COMMAND', 'cargo'), 'build', '--manifest-path',
                       str(binding / 'Cargo.toml'), '--features', 'contract-tests', '--message-format=json']
            built = subprocess.run(command, env=dict(os.environ, PYO3_PYTHON=sys.executable),
                                   text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=1200)
            artifacts = [json.loads(line) for line in built.stdout.splitlines() if line.startswith('{')]
            if built.returncode:
                diagnostics = ''.join(item['message'].get('rendered', '') for item in artifacts
                                      if item.get('reason') == 'compiler-message')
                raise AssertionError(built.stderr + '\n' + diagnostics)
            libraries = [path for item in artifacts if item.get('reason') == 'compiler-artifact'
                         and item['target']['name'] == 'aimux' for path in item['filenames']
                         if path.endswith(('.dylib', '.so', '.dll'))]
            if len(libraries) != 1:
                raise AssertionError('expected one native contract module: %r' % libraries)
            native_library = libraries[0]
        cls.folder = tempfile.TemporaryDirectory(prefix='aimux-python-vectors-')
        cls.addClassCleanup(cls.folder.cleanup)
        library = Path(cls.folder.name) / ('aimux.pyd' if sys.platform == 'win32' else 'aimux.so')
        shutil.copyfile(native_library, library)
        # The final component selects PyInit_aimux, while a distinct module key
        # keeps the installed production module and its owner guard intact.
        spec = importlib.util.spec_from_file_location('_aimux_contract.aimux', library)
        cls.native = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.native)
        cls.client = cls.native.Client()

    def test_shared_vectors(self):
        for path in self.paths:
            with self.subTest(scenario=path.stem):
                case = json.loads(path.read_text())
                expected = case.pop('expected')
                actual = json.loads(self.client.call('0', 'test.scenario', json.dumps(case)))
                if expected['abortObserved'] is None:
                    actual['abortObserved'] = None
                if actual['preflightError']:
                    actual['preflightError'] = self.error(actual['preflightError'])
                for key in ('frames', 'consumerFrames'):
                    terminal = next((frame for frame in expected[key] if frame['type'] == 'end'), {})
                    for frame in actual[key]:
                        if frame['type'] == 'end' and 'error' in frame:
                            if 'error' in terminal:
                                frame['error'] = self.error(frame['error'])
                            else:
                                del frame['error']
                self.assertEqual(actual, expected)

    def error(self, value):
        self.assertIsInstance(value['message'], str)
        self.assertIsInstance(value['error']['name'], str)
        return {'kind': value['kind'], 'code': value['code'], 'error': {'name': value['error']['name']}}

    def test_owner_teardown_precedes_runtime_shutdown(self):
        result = json.loads(self.client.call('0', 'test.teardown', '{}'))
        self.assertEqual(result, {'released': True, 'runtimeAlive': True})


if __name__ == '__main__':
    unittest.main()
