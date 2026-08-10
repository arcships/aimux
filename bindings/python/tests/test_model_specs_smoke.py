"""Offline smoke test for the ``get_model_specs`` entry point (B2).

Before the fix, ``__init__.py`` called ``_native_get_model_specs`` but never
imported that name from the native module, so ``aimux.get_model_specs(...)``
raised ``NameError`` at call time. The native fetch hits the network (anya2a
catalogue), so this test stubs the binding via ``unittest.mock`` — no real
HTTP is issued and the call path is exercised end-to-end.
"""

from unittest.mock import patch

import aimux


def test_get_model_specs_is_exposed():
    """The public entry point is present and callable."""
    assert hasattr(aimux, "get_model_specs")
    assert callable(aimux.get_model_specs)


def test_get_model_specs_native_binding_imported():
    """B2 regression guard: the native binding is imported, so calling the
    wrapper does not raise ``NameError``. The network fetch is stubbed."""
    assert hasattr(aimux, "_native_get_model_specs")

    with patch.object(aimux, "_native_get_model_specs", return_value='{"ok": true}') as mocked:
        result = aimux.get_model_specs(None)

    mocked.assert_called_once_with(None)
    assert result == {"ok": True}


def test_get_model_specs_forwards_source_url():
    """``source_url`` is forwarded to the native fetch (still offline)."""
    with patch.object(aimux, "_native_get_model_specs", return_value="{}") as mocked:
        aimux.get_model_specs("https://example.invalid/catalogue.json")

    mocked.assert_called_once_with("https://example.invalid/catalogue.json")
