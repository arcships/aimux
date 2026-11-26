DEFAULT_PIN = object()


def _mismatched_types(types, expected_hashes):
    if not expected_hashes or not isinstance(types, list) or not types:
        return None
    actual = {}
    for entry in types:
        if isinstance(entry, dict):
            type_id = entry.get("id")
            schema_hash = entry.get("schemaHash")
            if isinstance(type_id, str) and isinstance(schema_hash, str):
                actual[type_id] = schema_hash
    return [
        type_id
        for type_id, expected_hash in expected_hashes.items()
        if actual.get(type_id) != expected_hash
    ]


def verify_manifest_contract(
    description,
    *,
    actual_digest,
    expected_abi,
    expected_ops,
    expected_digest=DEFAULT_PIN,
    compile_time_abi,
    compile_time_ops,
    compile_time_digest,
    python_abi=None,
    python_ops=None,
    python_digest=None,
    type_hashes,
):
    if python_abi is None:
        python_abi = compile_time_abi
    if python_ops is None:
        python_ops = compile_time_ops
    if python_digest is None:
        python_digest = compile_time_digest
    if (
        expected_abi != compile_time_abi
        or expected_ops != compile_time_ops
        or expected_abi != python_abi
        or expected_ops != python_ops
    ):
        raise ImportError("aimux ABI/ops version differs from the Python binding")
    if description.get("abiVersion") != expected_abi or description.get("opsVersion") != expected_ops:
        raise ImportError("aimux manifest version differs from the native module")
    manifest_field_digest = description.get("digest")
    if manifest_field_digest is not None and manifest_field_digest != actual_digest:
        raise ImportError("aimux manifest digest differs from the native module")
    if expected_digest is None:
        return description
    if expected_digest is not DEFAULT_PIN:
        if actual_digest != expected_digest:
            raise ImportError(
                f"aimux manifest digest {actual_digest!r} differs from consumer pin {expected_digest!r}"
            )
        return description
    if actual_digest == compile_time_digest and actual_digest == python_digest:
        return description
    mismatches = _mismatched_types(description.get("types"), type_hashes)
    if mismatches is not None and not mismatches:
        return description
    target_digest = python_digest if actual_digest != python_digest else compile_time_digest
    detail = ""
    if mismatches:
        detail = f" (incompatible types: {', '.join(mismatches[:5])})"
    raise ImportError(
        f"aimux manifest digest {actual_digest!r} differs from expected {target_digest!r}{detail}"
    )
