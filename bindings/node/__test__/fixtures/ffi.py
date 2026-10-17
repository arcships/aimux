"""Execute JSON vectors through the installed C ABI for addon parity tests."""
import ctypes
import json
import sys

lib = ctypes.CDLL(sys.argv[1])
lib.aimux_call.argtypes = [ctypes.c_uint64, ctypes.c_char_p, ctypes.c_char_p,
                          ctypes.c_uint64, ctypes.POINTER(ctypes.c_void_p)]
lib.aimux_call.restype = ctypes.c_void_p
lib.aimux_error_json.argtypes = [ctypes.c_void_p]
lib.aimux_error_json.restype = ctypes.c_void_p
lib.aimux_error_free.argtypes = [ctypes.c_void_p]
lib.aimux_free_string.argtypes = [ctypes.c_void_p]
lib.aimux_drop.argtypes = [ctypes.c_uint64]
handles = {}


def resolve(value):
    if isinstance(value, str) and value.startswith('$'):
        return handles[value[1:]]
    if isinstance(value, list):
        return [resolve(item) for item in value]
    if isinstance(value, dict):
        return {key: resolve(item) for key, item in value.items()}
    return value


results = []
try:
    for vector in json.load(sys.stdin):
        output = ctypes.c_void_p()
        error = lib.aimux_call(int(resolve(vector.get('handle', '0'))),
                              vector['op'].encode(),
                              json.dumps(resolve(vector.get('params', {}))).encode(),
                              0, ctypes.byref(output))
        if error:
            data = lib.aimux_error_json(error)
            try:
                results.append({'error': json.loads(ctypes.string_at(data))})
            finally:
                lib.aimux_free_string(data)
                lib.aimux_error_free(error)
        else:
            try:
                result = json.loads(ctypes.string_at(output))
            finally:
                lib.aimux_free_string(output)
            if 'save' in vector:
                handles[vector['save']] = result if isinstance(result, str) else result['handle']
                result = ('$' + vector['save'] if isinstance(result, str)
                          else dict(result, handle='$' + vector['save']))
            results.append({'result': result})
finally:
    for handle in handles.values():
        lib.aimux_drop(int(handle))
print(json.dumps(results))
