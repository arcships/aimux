#!/usr/bin/env python3
"""Round-trip every DTO vector without loading the native extension."""
import importlib.util
import json
from pathlib import Path
import sys

from pydantic import TypeAdapter

binding = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location(
    '_aimux_dto_types', binding / 'python/aimux/_types.py',
)
types = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = types
spec.loader.exec_module(types)


def main():
    directory = binding.parents[1] / 'contract-tests/dto'
    adapters = {name: TypeAdapter(ty) for name, ty in types.DTO_TYPES.items()}
    for entry in json.loads((directory / 'index.json').read_text()):
        record = {'file': entry['file']}
        try:
            adapter = adapters[entry['type']]
            value = json.loads((directory / entry['file']).read_text())
            decoded = adapter.validate_python(value)
            record.update(ok=True, value=adapter.dump_python(decoded, mode='json', by_alias=True))
        except (ValueError, TypeError, KeyError) as error:
            record.update(ok=False, error=str(error))
        print(json.dumps(record, allow_nan=False))


if __name__ == '__main__':
    main()
