"""Pure-Python manifest version and per-type schema hash negotiation helpers."""


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
    expected_digest,
    compile_time_abi,
    compile_time_ops,
    compile_time_digest,
    type_hashes,
):
    """Validate ABI/ops versions, explicit consumer digest pins, and per-type schema hashes."""
    if expected_abi != compile_time_abi or expected_ops != compile_time_ops:
        raise ImportError("aimux ABI/ops version differs from the Python binding")
    if description.get("abiVersion") != expected_abi or description.get("opsVersion") != expected_ops:
        raise ImportError("aimux manifest version differs from the native module")
    if expected_digest is None:
        return description
    if expected_digest != compile_time_digest:
        if actual_digest != expected_digest:
            raise ImportError(
                f"aimux manifest digest {actual_digest!r} differs from consumer pin {expected_digest!r}"
            )
        return description
    if actual_digest == compile_time_digest:
        return description
    mismatches = _mismatched_types(description.get("types"), type_hashes)
    if mismatches is not None and not mismatches:
        return description
    detail = ""
    if mismatches:
        detail = f" (incompatible types: {', '.join(mismatches[:5])})"
    raise ImportError(
        f"aimux manifest digest {actual_digest!r} differs from expected {compile_time_digest!r}{detail}"
    )
