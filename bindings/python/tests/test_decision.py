"""Decision contract crosses Python -> PyO3 -> core -> HTTP -> Python dicts.

A thread-based server also verifies the native decision call releases the GIL.
"""
import base64
import json
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

import pytest
from aimux import APICallError, InvalidArgumentError, decide, jev_decision, describe_decision_model


def test_provider_handle_openai_decisions_answer_every_question():
    from aimux import create_provider
    contract = json.loads((Path(__file__).resolve().parents[3] /
        'aimux-providers/tests/fixtures/openai_decisions.json').read_text())
    captured = []

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            captured.append((self.path, self.headers['Authorization'],
                json.loads(self.rfile.read(int(self.headers['Content-Length'])))))
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.end_headers()
            self.wfile.write(json.dumps(contract['response']).encode())

        def log_message(self, *_):
            pass

    http = HTTPServer(('127.0.0.1', 0), Handler)
    worker = threading.Thread(target=http.serve_forever, daemon=True)
    worker.start()
    try:
        provider = create_provider('openai', 'test-key', base_url=f'http://127.0.0.1:{http.server_port}/v1')
        model = provider.decision_model('gpt-6-luna')
        assert describe_decision_model(model) == {
            'specificationVersion': 'v4', 'provider': 'openai', 'modelId': 'gpt-6-luna',
            'supportedQuestionTypes': ['choice', 'score', 'boolean']}
        result = decide(model, **contract['options'])
        assert captured == [('/v1/decisions', 'Bearer test-key', contract['request'])]
        assert list(result['answers']) == ['urgent', 'team', 'severity']
        assert result['answers']['urgent'] == {'type': 'boolean', 'probability': 0.925}
    finally:
        http.shutdown()
        worker.join()
        http.server_close()


def test_decision_uses_unified_runtime_recording(server, tmp_path):
    from aimux import init_recording, recording_flush, recording_stop
    base, _ = server
    init_recording(str(tmp_path))
    try:
        result = decide(jev_decision('test-key', 'jev-latest', base + '/v1/systemone'),
                        {'message': 'Billed twice'}, QUESTIONS, maxRetries=0)
    finally:
        recording_flush()
        recording_stop()
    records = [json.loads(line) for line in (tmp_path / 'recordings.jsonl').read_text().splitlines()]
    assert len(records) == 1
    record = records[0]
    assert record['complete'] is True
    assert record['input']['operation'] == 'decision'
    assert record['input']['options']['questions'] == QUESTIONS
    assert record['outcome']['decisionResult']['answers'] == result['answers']
    assert record['exchanges'][0]['attempt'] == 1
    assert 'test-key' not in json.dumps(record)

FIXTURE = json.loads((Path(__file__).resolve().parents[3] / 'aimux-providers/tests/fixtures/jev_systemone.json').read_text())['response']
QUESTIONS = {
    'is_urgent': {'type': 'boolean', 'instructions': 'The message conveys urgency or time-sensitivity'},
    'department': {'type': 'choice', 'instructions': 'Which team should handle this', 'criteria': {label: None for label in ['billing', 'technical', 'sales']}},
    'frustration': {'type': 'score', 'instructions': 'How frustrated the customer appears', 'criteria': ['Calm, just stating facts', 'Frustrated but civil', 'Very angry, strong language']},
}


@pytest.mark.parametrize('usage', [{}, {'input_tokens': None, 'output_tokens': None},
                                 {'input_tokens': 12}, {'output_tokens': 7}])
def test_optional_usage_counts_preserve_answers(server, monkeypatch, usage):
    monkeypatch.setitem(FIXTURE, 'usage', usage)
    base, _ = server
    result = decide(jev_decision('test-key', 'jev-latest', base + '/v1/systemone'),
                    {'message': 'Billed twice'}, QUESTIONS, maxRetries=0)
    assert len(result['answers']) == 3
    # An unknown count is absent, as the AI SDK's undefined is in JSON.
    expected = {key: usage[wire] for key, wire in
                [('inputTokens', 'input_tokens'), ('outputTokens', 'output_tokens')]
                if usage.get(wire) is not None}
    assert result['usage'] == expected
    assert result['response']['body']['usage'] == usage


@pytest.fixture
def server():
    captures = []

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            captures.append((self.path, {key.lower(): value for key, value in self.headers.items()}, request))
            body = json.dumps(FIXTURE if self.path == '/v1/systemone' else dict(FIXTURE, answers={})).encode()
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *args):
            pass

    http = HTTPServer(('127.0.0.1', 0), Handler)
    # Bound waits keep a GIL regression from hanging the test process forever.
    http.timeout = 2
    thread = threading.Thread(target=http.serve_forever, daemon=True)
    thread.start()
    yield f'http://127.0.0.1:{http.server_port}', captures
    http.shutdown()
    http.server_close()
    thread.join(timeout=2)


def test_decide_all_types_and_raw_response(server):
    base, captures = server
    model = jev_decision('test-key', 'jev-latest', base + '/v1/systemone')
    result = decide(model, {'message': 'Billed twice'}, QUESTIONS, maxRetries=0, timeout={'totalMs': 2000})
    assert result['answers']['is_urgent'] == {'type': 'boolean', 'probability': 1.0}
    assert result['answers']['department']['choice'] == 'technical'
    assert result['answers']['frustration']['score'] == 1.0
    assert result['answers']['frustration']['probabilities'] == {'0': 0, '1': 1.0, '2': 0}
    assert result['usage'] == {'inputTokens': 392, 'outputTokens': 65, 'totalTokens': 457}
    assert result['response']['body'] == FIXTURE
    assert result['providerMetadata']['jev']['probabilitySource'] == 'native'
    assert captures[0][0] == '/v1/systemone'
    assert captures[0][1]['authorization'] == 'Bearer test-key'
    assert captures[0][2]['questions']['is_urgent']['type'] == 'noul'


def test_invalid_answer_uses_python_error_class(server):
    base, _ = server
    model = jev_decision('test-key', 'jev-latest', base + '/invalid')
    with pytest.raises(APICallError):
        decide(model, 'text', QUESTIONS, maxRetries=0, timeout={'totalMs': 2000})


@pytest.mark.parametrize('source', ['native', 'logit_scoring', 'model_estimate'])
def test_self_hosted_probability_source(server, source):
    base, _ = server
    model = jev_decision('test-key', 'jev-latest', base + '/v1/systemone', probability_source=source)
    result = decide(model, 'text', QUESTIONS, maxRetries=0, timeout={'totalMs': 2000})
    assert result['providerMetadata']['jev']['probabilitySource'] == source


def test_unknown_probability_source():
    with pytest.raises(InvalidArgumentError):
        jev_decision('test-key', 'jev-latest', probability_source='unknown')


def test_official_structured_contract_and_description():
    fixture = json.loads((Path(__file__).resolve().parents[3] / 'contract-tests/fixtures/decision-native.json').read_text())
    captures = []
    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            captures.append(json.loads(self.rfile.read(int(self.headers['Content-Length']))))
            body = json.dumps(fixture['response']).encode()
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        def log_message(self, *args): pass
    http = HTTPServer(('127.0.0.1', 0), Handler)
    thread = threading.Thread(target=http.serve_forever, daemon=True)
    thread.start()
    try:
        model = jev_decision('test-key', 'jev-latest', f'http://127.0.0.1:{http.server_port}/v1/systemone')
        assert describe_decision_model(model)['provider'] == 'jev'
        assert not captures
        request = fixture['request']
        result = decide(model, request['state'], request['questions'], maxRetries=0, timeout={'totalMs':2000})
        assert captures[0]['questions']['urgent']['criteria'] == request['questions']['urgent']['criteria']
        assert captures[0]['questions']['severity']['criteria'] == request['questions']['severity']['criteria']
        assert result['answers']['severity'] == {'type': 'score', 'score': 0.25, 'probabilities': {'0': 0.75, '1': 0.25}}
        assert result['rounding'] == {'probabilityDecimals': 2, 'scoreDecimals': 2}
    finally:
        http.shutdown()
        http.server_close()
        thread.join(timeout=2)


@pytest.mark.parametrize('as_bytes', [False, True])
def test_openai_images_native_values_and_score_descriptions(as_bytes):
    from aimux import create_provider
    contract = json.loads((Path(__file__).resolve().parents[3] /
        'contract-tests/fixtures/decision-openai-full.json').read_text())
    if as_bytes:
        # DecisionImage.data is a base64 string on the wire, as the AI SDK
        # writes bytes (provider-utils uint8-utils.ts:12-23); bytes given to
        # decide() are encoded to the same string.
        openai = contract['options']['providerOptions']['openai']
        openai['images'] = [
            dict(image, data=base64.b64decode(image['data']))
            for image in openai['images']
        ]
    captured = []

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            captured.append(json.loads(self.rfile.read(int(self.headers['Content-Length']))))
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.end_headers()
            self.wfile.write(json.dumps(contract['response']).encode())

        def log_message(self, *_):
            pass

    http = HTTPServer(('127.0.0.1', 0), Handler)
    worker = threading.Thread(target=http.serve_forever, daemon=True)
    worker.start()
    try:
        model = create_provider('openai', 'test-key', base_url=f'http://127.0.0.1:{http.server_port}/v1').decision_model('gpt-6-luna')
        result = decide(model, **contract['options'])
        assert captured == [contract['request']]
        assert result['answers']['choice'] == {'type': 'choice', 'choice': 'boolean_true',
                                               'probabilities': {'boolean_true': 0.75, 'text_true': 0.25}}
        assert result['providerMetadata']['openai']['answers']['choice']['value'] is True
        assert result['answers']['score'] == {'type': 'score', 'score': 0.75, 'probabilities': {'0': 0.25, '1': 0.75}}
    finally:
        http.shutdown()
        worker.join()
        http.server_close()
