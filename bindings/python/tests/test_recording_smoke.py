"""Offline smoke test for the recording + mock-replay entry points (T5).

These were registered on the native module but never re-exported from the
Python package. Pure offline: ``init_recording`` writes to disk and
``mock_replay`` needs valid recordings, so we only assert the entry points are
exposed and callable — we never invoke them.
"""

import aimux

_ENTRIES = [
    "init_recording",
    "init_recording_ring",
    "recording_stop",
    "recording_flush",
    "recording_try_flush",
    "mock_replay",
]


def test_recording_entries_exposed():
    """Every recording/replay entry point is present and callable."""
    for name in _ENTRIES:
        assert hasattr(aimux, name), f"missing entry point: {name}"
        assert callable(getattr(aimux, name)), f"not callable: {name}"


def test_recording_entries_in_all():
    """Every recording/replay entry point is part of the public API."""
    for name in _ENTRIES:
        assert name in aimux.__all__, f"missing from __all__: {name}"


def test_init_recording_ring_no_arg_uses_default():
    """Omitting cap uses the library default capacity and does not raise.

    The ring recorder is in-memory (no disk I/O), so invoking it here is safe;
    ``recording_stop`` resets the global recorder afterward.
    """
    aimux.init_recording_ring()
    aimux.recording_stop()


def test_recorder_init_failure_raises_recording_error(tmp_path):
    """A failed recorder initialization is a separate, typed error.

    A parent path that is a regular file fails during initialization with
    ``Init``; no recorder is installed, so a checked flush remains a no-op.
    """
    import pytest

    aimux.recording_stop()
    aimux.recording_try_flush()  # nothing recording: nothing to flush

    # Parent path is a regular file: init itself raises (code "Init"); the
    # recorder is not silently degraded and discovered at the first flush.
    blocker = tmp_path / "occupied"
    blocker.write_text("x")
    try:
        with pytest.raises(aimux.RecordingError) as ei:
            aimux.init_recording(str(blocker / "sub"))
        assert ei.value.code == "Init"
        # A recorder failure is not an AiMuxError failure: separate type.
        assert not isinstance(ei.value, aimux.AimuxError)
        aimux.recording_try_flush()  # nothing installed: still a success
    finally:
        aimux.recording_stop()
