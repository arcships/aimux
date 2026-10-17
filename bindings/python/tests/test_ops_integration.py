"""Offline checks against the real assembled RFC-0039 runtime.

Build/install a production wheel, then set AIMUX_REAL_OPS=1. The isolated
adapter runner deliberately does not enable these tests.
"""
import os
import unittest

if os.environ.get('AIMUX_REAL_OPS') == '1':
    import aimux
    from aimux import ops


@unittest.skipUnless(os.environ.get('AIMUX_REAL_OPS') == '1', 'requires assembled shared ops runtime')
class RealOps(unittest.TestCase):
    def test_manifest_pin_and_describe(self):
        client = aimux.Client()
        self.assertEqual(aimux.verify_contract()['opsVersion'], ops.OPS_VERSION)
        self.assertEqual(aimux.manifest_digest(), aimux.EXPECTED_MANIFEST_DIGEST)
        self.assertIsInstance(client.call(ops.OP_RUNTIME_DESCRIBE), dict)

    def test_unknown_op_is_protocol_error(self):
        with self.assertRaises(aimux.ProtocolError) as error:
            aimux.Client().call('not.a.registered.operation')
        self.assertEqual(error.exception.code, 300)

    def test_unknown_handle_is_binding_error(self):
        with self.assertRaises(aimux.BindingError) as error:
            aimux.Client().call(ops.OP_MODEL_INFO, handle='18446744073709551615')
        self.assertEqual(error.exception.code, 203)

    def test_drop_unknown_and_global_is_idempotent(self):
        client = aimux.Client()
        for handle in ('0', '18446744073709551615'):
            for _ in range(2):
                client.call(ops.OP_HANDLE_DROP, handle=handle)

    def provider(self, client=None):
        client = client or aimux.Client()
        return client.adopt(client.call(ops.OP_PROVIDER_CREATE, {
            'package': 'openai', 'settings': {'apiKey': 'offline-test-key'},
        })['handle'])

    def test_generated_pin_rejects_drift(self):
        with self.assertRaises(ImportError):
            aimux.verify_contract('outdated-generated-metadata')

    def test_all_model_kinds_round_trip_and_child_survives_provider(self):
        with self.provider() as provider:
            models = [provider.client.adopt(provider.call(ops.OP_PROVIDER_MODEL, {
                'method': method, 'modelId': 'offline-model',
            })['handle']) for method in (
                'languageModel', 'embeddingModel', 'imageModel', 'speechModel',
                'transcriptionModel', 'decision',
            )]
        try:
            for model in models:
                self.assertIsInstance(model.call(ops.OP_MODEL_INFO), dict)
        finally:
            for model in models: model.close()

    def test_clients_in_same_interpreter_share_handles(self):
        with self.provider() as provider:
            other = aimux.Client()
            result = other.call(ops.OP_PROVIDER_MODEL, {
                'method': 'languageModel', 'modelId': 'offline-model',
            }, handle=provider)
            with other.adopt(result['handle']) as model:
                self.assertIsInstance(model.call(ops.OP_MODEL_INFO), dict)

    def test_drop_is_idempotent_and_invalid_handle_does_not_leak_kind(self):
        provider = self.provider()
        model = provider.client.adopt(provider.call(ops.OP_PROVIDER_MODEL, {
            'method': 'languageModel', 'modelId': 'offline-model',
        })['handle'])
        provider.close()
        model.close()
        model.close()
        with self.assertRaises(aimux.BindingError) as error:
            model.call(ops.OP_MODEL_INFO)
        self.assertEqual(error.exception.code, 203)
        self.assertIsNone(error.exception.error.get('actual'))

    def test_gc_queues_handle_cleanup(self):
        import gc
        import time
        client = aimux.Client()
        provider = self.provider(client)
        handle = provider.id
        del provider
        gc.collect()
        # provider.model is provider-scoped and creates no child on invalid input.
        for _ in range(100):
            try:
                result = client.call(ops.OP_PROVIDER_MODEL, {'method': 'languageModel', 'modelId': 'offline'}, handle=handle)
                client.adopt(result['handle']).close()
            except aimux.BindingError as error:
                self.assertEqual(error.code, 203)
                break
            time.sleep(0.01)
        else:
            self.fail('GC cleanup did not remove provider')

    def test_control_dtos_reject_unknown_fields(self):
        for op, params in [(ops.OP_RUNTIME_DESCRIBE, {'unknown': True}),
                           (ops.OP_PROVIDER_CREATE, {'package': 'openai', 'unknown': True})]:
            with self.subTest(op=op), self.assertRaises(aimux.ProtocolError) as error:
                aimux.Client().call(op, params)
            self.assertEqual(error.exception.code, 301)

    def test_wrong_handle_kind_rejected_before_provider_call(self):
        with self.provider() as provider:
            with self.assertRaises(aimux.BindingError) as error:
                provider.call(ops.OP_TEXT_GENERATE, {'prompt': 'hi'})
            self.assertEqual(error.exception.code, 203)

    def test_binary_push_uses_typed_service_and_enforces_byte_limit(self):
        client = aimux.Client()
        with self.assertRaises(aimux.BindingError) as error:
            client.operation_push('18446744073709551615', b'\0\0')
        self.assertEqual(error.exception.code, 203)
        for payload in (b'x' * (1024 * 1024 + 1), memoryview(b'x' * (1024 * 1024 + 1))):
            with self.assertRaises(aimux.ProtocolError) as error:
                client.operation_push('0', payload)
            self.assertEqual(error.exception.code, 305)
        with self.assertRaises(aimux.ProtocolError) as error:
            client.call(ops.OP_OPERATION_PUSH)
        self.assertEqual(error.exception.code, 300)

    def test_secondary_abort_is_owner_typed(self):
        with self.provider() as provider:
            with self.assertRaises(aimux.BindingError) as error:
                provider.call(ops.OP_PROVIDER_INVOKE, {'method': 'listModels', 'abort': provider.id})
            self.assertEqual(error.exception.code, 203)

    def test_stream_validation_error_precedes_acceptance(self):
        with self.assertRaises(aimux.BindingError):
            aimux.Client().stream(ops.OP_TEXT_STREAM, {'prompt': 'hi'}, handle='18446744073709551615')

    def test_recording_domain_comes_from_real_core(self):
        import tempfile
        from pathlib import Path
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'file'
            path.write_text('blocks directory creation')
            with self.assertRaises(aimux.RecordingError) as error:
                aimux.Client().call(ops.OP_RECORDER_CREATE, {
                    'backend': {'type': 'jsonl', 'directory': str(path / 'child')},
                    'capturePolicy': 'full',
                })
        self.assertEqual(error.exception.kind, 'recording')

    def test_recorder_round_trip(self):
        client = aimux.Client()
        with client.adopt(client.call(ops.OP_RECORDER_CREATE, {
            'backend': {'type': 'ring', 'capacity': 4}, 'capturePolicy': 'full',
        })['handle']) as recorder:
            self.assertIsInstance(recorder.call(ops.OP_RECORDER_STATS), dict)
            recorder.call(ops.OP_RECORDER_FLUSH)
            self.assertIsInstance(recorder.call(ops.OP_RECORDER_EXPORT)['jsonl'], str)

    @unittest.skipUnless(hasattr(os, 'mkfifo'), 'requires POSIX FIFOs')
    def test_real_constructor_io_releases_gil(self):
        from gil_ops import recording
        from test_ops_native import heartbeat
        with recording('create') as invoke:
            recorder = heartbeat(invoke)
            self.assertIsInstance(recorder.id, str)

    def test_old_constructors_are_absent(self):
        for name in ('openai', 'deepseek', 'anthropic', 'openai_embedding', 'DecisionModel', 'init_recording'):
            self.assertFalse(hasattr(aimux, name), name)


@unittest.skipUnless(os.environ.get('AIMUX_REAL_OPS') == '1' and os.environ.get('AIMUX_HTTP_TESTS') == '1',
                     'requires localhost HTTP/WS fixtures')
class RealProviderRoundTrips(unittest.TestCase):
    def test_modalities(self):
        from gil_ops import MODALITIES, modality
        expected = {'embedding': 'embeddings', 'embedding_many': 'embeddings', 'speech': 'audio',
                    'image': 'images', 'transcription': 'text', 'reranking': 'ranking',
                    'search': 'results', 'upload_file': 'providerReference'}
        for name in MODALITIES:
            with self.subTest(name=name), modality(name) as invoke:
                result = invoke()
                self.assertIn(expected[name], result)
                self.assertEqual(__import__('json').loads(__import__('json').dumps(result)), result)

    def test_text_and_streams(self):
        from gil_ops import language
        for op in (ops.OP_TEXT_GENERATE, ops.OP_TEXT_GENERATE_OBJECT, ops.OP_TEXT_GENERATE_OPEN_AI, ops.OP_TEXT_CONSUME,
                   ops.OP_TEXT_STREAM, ops.OP_TEXT_STREAM_OPEN_AI):
            with self.subTest(op=op), language(op) as invoke:
                result = invoke()
                self.assertIsInstance(result, list if op in (ops.OP_TEXT_STREAM, ops.OP_TEXT_STREAM_OPEN_AI) else dict)
                if op in (ops.OP_TEXT_STREAM, ops.OP_TEXT_STREAM_OPEN_AI):
                    # The Python bridge consumes End: success stops iteration,
                    # while terminal failures raise typed exceptions.
                    self.assertTrue(result)
                    self.assertTrue(all(frame['type'] == 'event' for frame in result))
                    events = [frame['event'] for frame in result]
                    if op == ops.OP_TEXT_STREAM:
                        text = ''.join(event['text'] for event in events if event['type'] == 'text-delta')
                    else:
                        text = ''.join(choice['delta'].get('content', '')
                                       for event in events for choice in event['choices'])
                    self.assertEqual(text, 'Hello world')
                elif op == ops.OP_TEXT_GENERATE_OBJECT:
                    self.assertEqual(result['object'], {'answer': 'hello'})
                elif op == ops.OP_TEXT_GENERATE_OPEN_AI:
                    self.assertEqual(result['choices'][0]['message']['content'],
                                     'Rust is a systems programming language.')
                else:
                    self.assertEqual(result['text'], 'Hello world' if op == ops.OP_TEXT_CONSUME
                                     else 'Rust is a systems programming language.')

    def test_video_and_decision(self):
        from gil_ops import video, decision
        for factory in (video, decision):
            with self.subTest(factory=factory.__name__), factory() as invoke:
                self.assertIsInstance(invoke(), dict)

    def test_realtime_push_and_poll(self):
        from gil_ops import model_at
        from ops_fixtures import slow_ws
        with slow_ws(0) as url, model_at(url, 'transcriptionModel', 'gpt-realtime-whisper') as model:
            with model.client.adopt(model.call(ops.OP_TRANSCRIPTION_START)) as operation:
                operation.push(b'\0\0')
                part = operation.call(ops.OP_OPERATION_NEXT, {'timeoutMs': 5000})
                self.assertEqual(part['type'], 'part')
                operation.call(ops.OP_OPERATION_INPUT_DONE)
                operation.call(ops.OP_OPERATION_CANCEL)
                self.assertIsInstance(operation.call(ops.OP_OPERATION_STATUS), dict)
