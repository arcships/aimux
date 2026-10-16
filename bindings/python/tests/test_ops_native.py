"""Native boundary tests using the isolated dispatcher double.

Run with run_native_contract.py. Real provider/ops integration is separate.
"""
import gc
import json
import os
import threading
import time
import unittest
import weakref

if os.environ.get('AIMUX_OPS_DOUBLE') == '1':
    import aimux
    from aimux import ops


def heartbeat(operation):
    ticks = []
    stop = threading.Event()
    ready = threading.Event()

    def beat():
        ready.set()
        while not stop.wait(0.005):
            ticks.append(time.monotonic())

    thread = threading.Thread(target=beat)
    thread.start()
    ready.wait()
    started = time.monotonic()
    try:
        result = operation()
        ended = time.monotonic()
    finally:
        stop.set()
        thread.join(2)
    assert ended - started >= 0.12, (started, ended)
    # A tick well inside the wait proves concurrency without asserting CPU
    # throughput on an oversubscribed build host. Exclude Python entry/exit.
    assert any(started + 0.02 < tick < ended - 0.02 for tick in ticks), ticks
    return result


@unittest.skipUnless(os.environ.get('AIMUX_OPS_DOUBLE') == '1', 'requires isolated native build')
class NativeBoundary(unittest.TestCase):
    def setUp(self):
        self.client = aimux.Client()

    def stream(self, **params):
        return self.client.stream(ops.OP_TEXT_STREAM, params, handle='0')

    def stats(self):
        return self.client.call('test.stats')

    def wait_inactive(self):
        for _ in range(100):
            if self.stats()['active'] == 0:
                return
            time.sleep(0.01)
        self.fail('stream task leaked')

    @unittest.skipUnless(os.name == 'posix', 'POSIX GIL-holding control')
    def test_heartbeat_rejects_a_gil_holding_native_call(self):
        import ctypes
        # PyDLL deliberately retains the GIL, unlike CDLL. This checks that
        # the heartbeat harness detects a real native blocking regression.
        sleep = ctypes.PyDLL(None).usleep
        sleep.argtypes = [ctypes.c_uint]
        sleep.restype = ctypes.c_int
        with self.assertRaises(AssertionError):
            heartbeat(lambda: sleep(160000))

    def test_gil_heartbeat_every_blocking_call_family(self):
        operations = [
            ops.OP_TEXT_GENERATE, ops.OP_TEXT_GENERATE_OBJECT, ops.OP_TEXT_CONSUME, ops.OP_TEXT_GENERATE_OPEN_AI,
            ops.OP_EMBEDDING_EMBED, ops.OP_EMBEDDING_EMBED_MANY, ops.OP_IMAGE_GENERATE, ops.OP_SPEECH_GENERATE,
            ops.OP_TRANSCRIPTION_GENERATE, ops.OP_RERANKING_RERANK, ops.OP_VIDEO_GENERATE, ops.OP_SEARCH_SEARCH,
            ops.OP_PROVIDER_CREATE, ops.OP_PROVIDER_INVOKE, ops.OP_CATALOGUE_FETCH, ops.OP_RECORDER_CREATE,
            ops.OP_TRANSCRIPTION_START, ops.OP_OPERATION_NEXT, ops.OP_RECORDER_FLUSH, ops.OP_FILES_UPLOAD,
        ]
        for op in operations:
            with self.subTest(op=op):
                result = heartbeat(lambda: self.client.call(op))
                if 'handle' in result:
                    self.client.adopt(result['handle']).close()

    def test_stream_construction_releases_gil(self):
        stream = heartbeat(lambda: self.stream(acceptDelayMs=160))
        stream.close()

    def test_stream_poll_releases_gil(self):
        with self.stream(delayMs=160) as stream:
            self.assertEqual(heartbeat(lambda: next(stream))['seq'], '0')

    def test_join_releases_gil(self):
        with self.stream(count=0, delayMs=160) as stream:
            heartbeat(stream.join)
            self.assertEqual(list(stream), [])

    def test_close_releases_gil_during_teardown(self):
        stream = self.stream(count=100000, dropDelayMs=160)
        time.sleep(0.04)
        heartbeat(stream.close)
        with self.assertRaises(aimux.RequestAbortedError):
            next(stream)

    def test_full_queue_close_does_not_deadlock(self):
        stream = self.stream(count=100000)
        time.sleep(0.1)
        started = time.monotonic()
        stream.close()
        self.assertLess(time.monotonic() - started, 1)
        stream.close()
        self.wait_inactive()

    def test_byte_budget_backpressure_is_cancellable(self):
        stream = self.stream(count=100000, bytes=900000)
        time.sleep(0.15)
        stream.close()
        self.wait_inactive()

    def test_single_event_limit_raises_protocol_error(self):
        stream = self.stream(bytes=1024*1024)
        try:
            with self.assertRaises(aimux.ProtocolError) as error:
                next(stream)
            self.assertEqual(error.exception.code, 305)
        finally:
            stream.close()

    def test_success_is_the_only_normal_exhaustion(self):
        stream = self.stream()
        self.assertEqual([frame['seq'] for frame in stream], ['0', '1'])
        with self.assertRaises(StopIteration):
            next(stream)
        stream.join()

    def test_late_cancel_preserves_success(self):
        stream = self.stream(count=1)
        stream.join()
        stream.cancel()
        self.assertEqual([frame['seq'] for frame in stream], ['0'])
        stream.close()
        self.assertEqual(list(stream), [])

    def test_success_does_not_cancel_a_shared_abort(self):
        abort = aimux.Abort()
        self.client.call('test.echo', {'delayMs': 0}, abort=abort)
        self.assertFalse(abort.cancelled)

    def test_python_serialization_errors_use_binding_domain(self):
        for params in [{'bytes': b'raw'}, {'bad': float('nan')}, {'bad': object()}]:
            with self.subTest(params=params), self.assertRaises(aimux.BindingError) as error:
                self.client.call('test.echo', params)
            self.assertEqual(error.exception.kind, 'binding')
        for arguments in [{'handle': 7}, {'op': 1}, {'abort': object()}, {'op': '\ud800'}]:
            with self.subTest(arguments=arguments), self.assertRaises(aimux.BindingError):
                self.client.call(**dict({'op': 'test.echo'}, **arguments))

    def test_full_queue_keeps_terminal_error(self):
        stream = self.stream(count=64, outcome='error')
        stream.join()
        delivered = 0
        try:
            with self.assertRaises(aimux.APICallError):
                while True:
                    next(stream)
                    delivered += 1
            self.assertEqual(delivered, 64)
        finally:
            stream.close()

    def test_recoverable_error_event_is_data(self):
        with self.stream(count=1, recoverableError=True) as stream:
            self.assertEqual(next(stream)['event']['type'], 'error')
            self.assertEqual(list(stream), [])

    def test_error_terminal_preserves_subtype(self):
        with self.stream(outcome='error', count=1) as stream:
            self.assertEqual(next(stream)['seq'], '0')
            for _ in range(2):
                with self.assertRaises(aimux.APICallError) as error:
                    next(stream)
                self.assertEqual(error.exception.status_code, 429)

    def test_cancelled_terminal_raises(self):
        with self.stream(outcome='cancelled', count=0) as stream:
            with self.assertRaises(aimux.RequestAbortedError):
                next(stream)

    def test_missing_terminal_is_protocol_failure(self):
        with self.stream(outcome='missing', count=0) as stream:
            with self.assertRaises(aimux.ProtocolError) as error:
                next(stream)
            self.assertEqual(error.exception.code, 304)

    def test_acceptance_error_is_synchronous(self):
        with self.assertRaises(aimux.ProtocolError):
            self.stream(reject=True)

    def test_context_exit_cancels(self):
        with self.stream(count=100000) as stream:
            next(stream)
        with self.assertRaises(aimux.RequestAbortedError):
            next(stream)
        self.wait_inactive()

    def test_explicit_abort_interrupts_call(self):
        abort = aimux.Abort()
        timer = threading.Timer(0.1, abort.cancel)
        timer.start()
        started = time.monotonic()
        try:
            with self.assertRaises(aimux.RequestAbortedError):
                self.client.call(ops.OP_TEXT_GENERATE, {'delayMs': 10000}, abort=abort)
        finally:
            timer.join()
        self.assertLess(time.monotonic() - started, 1)
        self.assertTrue(abort.cancelled)

    def test_explicit_abort_interrupts_stream(self):
        abort = aimux.Abort()
        stream = self.client.stream(ops.OP_TEXT_STREAM, {'delayMs': 10000}, handle='0', abort=abort)
        abort.cancel()
        try:
            with self.assertRaises(aimux.RequestAbortedError):
                next(stream)
        finally:
            stream.close()

    def test_keyboard_interrupt_cancels_calls(self):
        import _thread
        timer = threading.Timer(0.1, _thread.interrupt_main)
        timer.start()
        try:
            with self.assertRaises(KeyboardInterrupt):
                self.client.call(ops.OP_TEXT_GENERATE, {'delayMs': 10000})
        finally:
            timer.join()

    def test_keyboard_interrupt_cancels_poll(self):
        import _thread
        stream = self.stream(delayMs=10000)
        timer = threading.Timer(0.1, _thread.interrupt_main)
        timer.start()
        try:
            with self.assertRaises(KeyboardInterrupt):
                next(stream)
        finally:
            timer.join()
            stream.close()
        self.wait_inactive()

    def test_concurrent_consumer_is_protocol_error(self):
        stream = self.stream(delayMs=10000)
        errors = []
        ready = threading.Event()
        def consume():
            ready.set()
            try:
                next(stream)
            except aimux.RequestAbortedError:
                pass
            except BaseException as error:
                errors.append(error)
        thread = threading.Thread(target=consume)
        thread.start()
        ready.wait()
        time.sleep(0.05)
        try:
            with self.assertRaises(aimux.ProtocolError) as error:
                next(stream)
            self.assertEqual(error.exception.code, 303)
        finally:
            stream.cancel()
            thread.join(2)
            stream.close()
        self.assertFalse(thread.is_alive())
        self.assertEqual(errors, [])

    def test_gc_cancels_unconsumed_stream(self):
        stream = self.stream(count=100000)
        ref = weakref.ref(stream)
        del stream
        gc.collect()
        self.assertIsNone(ref())
        self.wait_inactive()

    def test_handle_context_and_gc_drop(self):
        before = self.stats()['handles']
        result = self.client.call(ops.OP_PROVIDER_CREATE, {'delayMs': 0})
        with self.client.adopt(result['handle']) as provider:
            self.assertEqual(self.stats()['handles'], before + 1)
        self.assertEqual(self.stats()['handles'], before)
        provider.close()
        with self.assertRaises(aimux.BindingError) as error:
            provider.call(ops.OP_MODEL_INFO)
        self.assertEqual(error.exception.code, 203)
        leaked = self.client.adopt(self.client.call(ops.OP_PROVIDER_CREATE, {'delayMs': 0})['handle'])
        del leaked
        gc.collect()
        for _ in range(100):
            if self.stats()['handles'] == before:
                break
            time.sleep(0.01)
        self.assertEqual(self.stats()['handles'], before)

    def test_owner_is_shared_between_clients_in_an_interpreter(self):
        first = self.client.call('test.echo', {'delayMs': 0})['owner']
        self.assertGreater(first, 0)
        self.assertEqual(aimux.Client().call('test.echo', {'delayMs': 0})['owner'], first)

    def test_owner_cannot_be_forged_with_python_monkeypatch(self):
        import _xxsubinterpreters
        actual = self.client.call('test.echo', {'delayMs': 0})['owner']
        original = _xxsubinterpreters.get_current
        _xxsubinterpreters.get_current = lambda: 99999999
        try:
            self.assertEqual(aimux.Client().call('test.echo', {'delayMs': 0})['owner'], actual)
        finally:
            _xxsubinterpreters.get_current = original

    def test_wire_errors_use_binding_domain(self):
        with self.assertRaises(aimux.BindingError) as error:
            self.client._native.call('0', ops.OP_TEXT_GENERATE, '{bad')
        self.assertEqual(error.exception.code, 202)
        self.assertEqual(error.exception.argument, 'params_json')
        for handle in ['-1', ' 1', '', '18446744073709551616', '1x']:
            with self.subTest(handle=handle), self.assertRaises(aimux.ProtocolError):
                self.client._native.call(handle, ops.OP_MODEL_INFO)

    def test_multimodal_wire_round_trip(self):
        params = {
            'delayMs': 0, 'values': ['中文', '😀'], 'audio': {'type':'base64','value':'AAEC'},
            'providerOptions': {'vendor': {'open': [None, True, 1.5]}},
        }
        for op in [ops.OP_EMBEDDING_EMBED_MANY, ops.OP_IMAGE_GENERATE, ops.OP_SPEECH_GENERATE, ops.OP_TRANSCRIPTION_GENERATE,
                   ops.OP_RERANKING_RERANK, ops.OP_VIDEO_GENERATE, ops.OP_SEARCH_SEARCH, ops.OP_FILES_UPLOAD]:
            with self.subTest(op=op):
                self.assertEqual(self.client.call(op, params)['params'], params)

    def test_nested_errors_from_native_envelope(self):
        api = dict(name='AI_APICallError', statusCode=429, message='slow', isRetryable=True)
        payload = dict(name='AI_RetryError', reason='maxRetriesExceeded', errors=[api, api])
        with self.assertRaises(aimux.RetryError) as error:
            self.client.call(ops.OP_TEXT_GENERATE, {'delayMs': 0, 'failure': payload})
        self.assertEqual(len(error.exception.errors), 2)
        self.assertIsInstance(error.exception.last_error, aimux.APICallError)
        payload = dict(name='AI_ToolCallRepairError', originalError=api,
                       cause={'name':'AI_JSONParseError','message':'bad'})
        with self.assertRaises(aimux.ToolCallRepairError) as error:
            self.client.call(ops.OP_TEXT_GENERATE, {'delayMs': 0, 'failure': payload})
        self.assertIsInstance(error.exception.__cause__, aimux.JSONParseError)

    def test_manifest_checks(self):
        self.assertEqual(aimux.verify_contract()['opsVersion'], ops.OPS_VERSION)
        self.assertEqual(aimux.manifest_digest(), ops.MANIFEST_DIGEST)
        with self.assertRaises(ImportError):
            aimux.verify_contract('a different consumer digest')
        aimux.verify_contract(aimux.manifest_digest())


if __name__ == '__main__':
    unittest.main()
