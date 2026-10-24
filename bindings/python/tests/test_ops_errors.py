"""Envelope-only tests run without a native extension or provider transport."""
import importlib
from pathlib import Path
import sys
import types
import unittest

package = types.ModuleType('_aimux_error_tests')
package.__path__ = [str(Path(__file__).parents[1] / 'python/aimux')]
sys.modules[package.__name__] = package
e = importlib.import_module(package.__name__ + '._errors')


def envelope(payload, kind='aimux', code=1, **extra):
    return dict(kind=kind, code=code, message='canonical message', error=payload, **extra)


API = dict(name='AI_APICallError', message='rate limited', statusCode=429,
           responseHeaders={'retry-after-ms': '20'}, isRetryable=True)


class Errors(unittest.TestCase):
    def test_every_aimux_name_has_distinct_subtype(self):
        for name, cls in e._AIMUX_TYPES.items():
            with self.subTest(name=name):
                payload = {'name': name}
                if name == 'AI_RetryError':
                    payload['errors'] = []
                error = e.from_envelope(envelope(payload))
                self.assertIs(type(error), cls)
                self.assertIsInstance(error, e.AimuxError)

    def test_all_payload_fields_and_envelope_survive(self):
        value = envelope(API, retryAfterMs=20)
        error = e.from_envelope(value)
        self.assertIs(error.envelope, value)
        self.assertIs(error.error, API)
        self.assertEqual(error.code, 1)
        self.assertEqual(error.kind, 'aimux')
        self.assertEqual(error.status_code, 429)
        self.assertTrue(error.is_retryable)
        self.assertEqual(error.response_headers, {'retry-after-ms': '20'})
        self.assertEqual(error.retry_after_ms, 20)
        self.assertEqual(str(error), 'canonical message')

    def test_history_retains_typed_attempts_without_inventing_top_hint(self):
        value = envelope(dict(name='AI_RetryError', reason='maxRetriesExceeded', errors=[API, API]))
        error = e.from_envelope(value)
        self.assertIsInstance(error, e.RetryError)
        self.assertIsNone(error.retry_after_ms)
        self.assertEqual(error.reason, 'maxRetriesExceeded')
        self.assertEqual(len(error.errors), 2)
        self.assertIs(error.last_error, error.errors[-1])
        self.assertIs(error.__cause__, error.last_error)
        for attempt in error.errors:
            self.assertIsInstance(attempt, e.APICallError)
            self.assertEqual(attempt.response_headers['retry-after-ms'], '20')
            self.assertFalse(hasattr(attempt, 'retry_after_ms'))

    def test_enveloped_attempts_retain_their_hints(self):
        error = e.from_envelope(envelope(dict(name='AI_RetryError', errors=[envelope(API, retryAfterMs=20)])))
        self.assertEqual(error.last_error.retry_after_ms, 20)
        self.assertIsNone(error.retry_after_ms)

    def test_repair_preserves_cause_and_original(self):
        cause = dict(name='AI_JSONParseError', message='invalid JSON')
        error = e.from_envelope(envelope(dict(name='AI_ToolCallRepairError', originalError=API, cause=cause)))
        self.assertIsInstance(error.original_error, e.APICallError)
        self.assertIsInstance(error.cause, e.JSONParseError)
        self.assertIs(error.__cause__, error.cause)
        self.assertIs(error.error['originalError'], API)

    def test_transport_cause_chain_becomes_source_errors(self):
        os_error = dict(message='Connection refused (os error 111)')
        cause = dict(message='error sending request', cause=dict(message='tcp connect error', cause=os_error))
        error = e.from_envelope(envelope(dict(API, cause=cause)))
        self.assertIsInstance(error.__cause__, e.SourceError)
        self.assertEqual(str(error.__cause__), 'error sending request')
        self.assertEqual(str(error.__cause__.__cause__.__cause__), 'Connection refused (os error 111)')
        self.assertNotIsInstance(error.__cause__, e.AimuxError)

    def test_nested_aimux_cause_is_its_exception_class(self):
        cause = dict(name='AI_JSONParseError', message='Unexpected end of JSON input')
        error = e.from_envelope(envelope(dict(name='AI_InvalidResponseDataError', message='eof', cause=cause)))
        self.assertIsInstance(error.__cause__, e.JSONParseError)
        self.assertEqual(str(error.__cause__), 'Unexpected end of JSON input')

    def test_domain_errors_carry_their_cause(self):
        payload = dict(name='Init', path='recordings', cause=dict(message='permission denied'))
        error = e.from_envelope(envelope(payload, kind='recording', code=100))
        self.assertIsInstance(error, e.RecordingError)
        self.assertIsInstance(error.__cause__, e.SourceError)
        self.assertEqual(str(error.__cause__), 'permission denied')

    def test_domains_are_separate(self):
        for domain, cls in [('recording', e.RecordingError), ('binding', e.BindingError), ('protocol', e.ProtocolError)]:
            with self.subTest(domain=domain):
                error = e.from_envelope(envelope(dict(name='InvalidState', detail={'x': [1]}), domain, 304))
                self.assertIs(type(error), cls)
                self.assertNotIsInstance(error, e.AimuxError)
                self.assertEqual(error.detail, {'x': [1]})

    def test_unknown_aimux_payload_keeps_evidence(self):
        payload = dict(name='FutureError', extra={'nested': '证据'})
        error = e.from_envelope(envelope(payload))
        self.assertIs(type(error), e.AimuxError)
        self.assertIs(error.error, payload)

    def test_opaque_provider_data_is_not_an_exception(self):
        data = {'name': 'provider-object', 'value': 18446744073709551615}
        error = e.from_envelope(envelope(dict(API, data=data)))
        self.assertIs(error.data, data)


if __name__ == '__main__':
    unittest.main()
