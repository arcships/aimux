"""RFC-0039 §9.8: heartbeat during *each* blocking native entry.

Run with an installed wheel: python -m pytest tests/test_gil_release.py -v.
Each entry runs in a disposable process, so a held-GIL deadlock has a deadline.
HTTP/WS fixtures run in independent processes; FIFO recording exercises real
constructor I/O, flush barriers and writer joins without mocking native code.

ENTRY_POINTS is the API-specific inventory. A migrated binding can supply
an alternative table using AIMUX_GIL_ENTRY_TABLE=module:attribute. Keep the same
case names/categories; each value is a context manager yielding a zero-argument
blocking call. Missing entries fail instead of silently losing coverage.
"""

import importlib
import multiprocessing
import os
from pathlib import Path
import subprocess
import sys
import threading
import time

import pytest

DELAY = 0.5
TIMEOUT = 25


def _heartbeat(call):
    """Exclude edge scheduling and require progress throughout the wait."""
    stop = threading.Event()
    ready = threading.Event()
    ticks = []

    def ticker():
        ready.set()
        while not stop.wait(0.005):
            ticks.append(time.monotonic())

    thread = threading.Thread(target=ticker, daemon=True)
    thread.start()
    assert ready.wait(2), "heartbeat did not start"
    try:
        start = time.monotonic()
        result = call()
        end = time.monotonic()
    finally:
        stop.set()
        thread.join(timeout=2)
    elapsed = end - start
    assert elapsed >= DELAY * 0.7, "mock did not make this entry block"
    interior = [t for t in ticks if start + 0.03 < t < end - 0.03]
    assert len(interior) >= 10, "Python heartbeat stalled (GIL held)"
    gaps = [b - a for a, b in zip([start] + interior, interior + [end])]
    assert max(gaps) < DELAY * 0.6, "Python heartbeat stopped during native wait"
    return result


# The binding uses shared ops handles. Binary push is immediate bounded
# admission, so it has separate ResourceLimit tests instead of a blocking case.
from gil_ops import ENTRY_POINTS


def _entry_table():
    target = os.environ.get("AIMUX_GIL_ENTRY_TABLE")
    if not target:
        return ENTRY_POINTS
    module, attribute = target.split(":", 1)
    table = getattr(importlib.import_module(module), attribute)
    assert set(table) >= set(ENTRY_POINTS), "migrated table omits blocking entries"
    for name, (category, _) in ENTRY_POINTS.items():
        assert table[name][0] == category, "migrated category changed: " + name
    return table


@pytest.mark.parametrize("name", sorted(_entry_table()))
def test_blocking_entry_releases_gil(name):
    env = dict(os.environ, NO_PROXY="127.0.0.1,localhost", no_proxy="127.0.0.1,localhost")
    try:
        process = subprocess.run(
            [sys.executable, str(Path(__file__).resolve()), "--entry", name],
            env=env, capture_output=True, text=True, timeout=TIMEOUT,
        )
    except subprocess.TimeoutExpired:
        pytest.fail(name + ": child exceeded deadline (native wait or teardown deadlock)")
    if process.returncode == 77:
        pytest.skip(process.stdout.strip())
    assert process.returncode == 0, process.stdout + process.stderr


def test_heartbeat_detects_a_stalled_ticker():
    # A deliberately held GIL proves boundary ticks cannot give a false pass.
    def hold_gil():
        previous = sys.getswitchinterval()
        try:
            sys.setswitchinterval(2)
            until = time.monotonic() + DELAY
            while time.monotonic() < until:
                pass
        finally:
            sys.setswitchinterval(previous)

    with pytest.raises(AssertionError, match="heartbeat"):
        _heartbeat(hold_gil)


def test_heartbeat_accepts_a_released_gil():
    _heartbeat(lambda: time.sleep(DELAY))


if __name__ == "__main__" and sys.argv[1:2] == ["--entry"]:
    multiprocessing.set_start_method("spawn", force=True)
    try:
        with _entry_table()[sys.argv[2]][1]() as blocking_call:
            _heartbeat(blocking_call)
    except pytest.skip.Exception as skip:
        print(str(skip))
        sys.exit(77)
