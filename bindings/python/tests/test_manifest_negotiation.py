"""Pure-Python unit tests for manifest digest and per-type schemaHash negotiation."""
import importlib.util
from pathlib import Path
import unittest


def _load(name, relative_path):
    path = Path(__file__).resolve().parents[1] / relative_path
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


_manifest = _load("aimux_manifest_helper", "python/aimux/_manifest.py")
_ops = _load("aimux_generated_ops", "python/aimux/ops.py")


class ManifestNegotiationTests(unittest.TestCase):
    def _base_kwargs(self, *, actual_digest, expected_digest=None):
        if expected_digest is None:
            expected_digest = _ops.MANIFEST_DIGEST
        return dict(
            actual_digest=actual_digest,
            expected_abi=_ops.ABI_VERSION,
            expected_ops=_ops.OPS_VERSION,
            expected_digest=expected_digest,
            compile_time_abi=_ops.ABI_VERSION,
            compile_time_ops=_ops.OPS_VERSION,
            compile_time_digest=_ops.MANIFEST_DIGEST,
            type_hashes=_ops.TYPE_SCHEMA_HASHES,
        )

    def test_accepts_altered_packages_and_digest_with_intact_type_hashes(self):
        types = [
            {"id": type_id, "revision": "1", "schemaHash": schema_hash}
            for type_id, schema_hash in _ops.TYPE_SCHEMA_HASHES.items()
        ]
        self.assertGreater(len(types), 0)
        description = {
            "abiVersion": _ops.ABI_VERSION,
            "opsVersion": _ops.OPS_VERSION,
            "digest": "changed-digest-after-package-addition",
            "packages": ["openai", "synthetic-package"],
            "types": types,
        }
        result = _manifest.verify_manifest_contract(
            description,
            **self._base_kwargs(actual_digest=description["digest"]),
        )
        self.assertEqual(result, description)

    def test_rejects_mismatched_schema_hash_and_explicit_consumer_pin(self):
        types = [
            {"id": type_id, "revision": "1", "schemaHash": schema_hash}
            for type_id, schema_hash in _ops.TYPE_SCHEMA_HASHES.items()
        ]
        target_id = types[0]["id"]
        broken = [dict(types[0], schemaHash="0" * 64), *types[1:]]
        description = {
            "abiVersion": _ops.ABI_VERSION,
            "opsVersion": _ops.OPS_VERSION,
            "digest": "changed-digest",
            "types": broken,
        }
        with self.assertRaises(ImportError) as ctx:
            _manifest.verify_manifest_contract(
                description,
                **self._base_kwargs(actual_digest="changed-digest"),
            )
        self.assertIn(_ops.MANIFEST_DIGEST, str(ctx.exception))
        self.assertIn(target_id, str(ctx.exception))

        # Explicit consumer pin must reject even if type schema hashes match.
        intact = dict(description, types=types)
        with self.assertRaises(ImportError):
            _manifest.verify_manifest_contract(
                intact,
                **self._base_kwargs(
                    actual_digest="changed-digest",
                    expected_digest="explicit-consumer-pin",
                ),
            )


if __name__ == "__main__":
    unittest.main()
