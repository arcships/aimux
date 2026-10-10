"""Decision contract crosses Python -> PyO3 -> core -> HTTP -> Python dicts.

A thread-based server also verifies the native decision call releases the GIL.
"""
import json
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

import pytest
from aimux import APICallError, InvalidArgumentError, decide, jev_decision, decision_capabilities


def test_provider_handle_openai_decisions_preserve_partial_refusals():
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
        assert decision_capabilities(model)['min_choices'] == 2
        result = decide(model, **contract['options'])
        assert captured == [('/v1/decisions', 'Bearer test-key', contract['request'])]
        assert result['answers']['restricted'] == {'type': 'refusal'}
        assert result['answers']['urgent']['probability_true'] == 0.925
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
                        {'message': 'Billed twice'}, QUESTIONS, max_retries=0)
    finally:
        recording_flush()
        recording_stop()
    records = [json.loads(line) for line in (tmp_path / 'recordings.jsonl').read_text().splitlines()]
    assert len(records) == 1
    record = records[0]
    assert record['complete'] is True
    assert record['input']['operation'] == 'decision'
    assert record['input']['options']['questions'] == QUESTIONS
    assert record['outcome']['decision_result']['answers'] == result['answers']
    assert record['exchanges'][0]['attempt'] == 1
    assert 'test-key' not in json.dumps(record)

FIXTURE = json.loads((Path(__file__).resolve().parents[3] / 'aimux-providers/tests/fixtures/jev_systemone.json').read_text())['response']
QUESTIONS = [
    {'id': 'is_urgent', 'type': 'boolean', 'instructions': 'The message conveys urgency or time-sensitivity'},
    {'id': 'department', 'type': 'choice', 'instructions': 'Which team should handle this', 'options': [{'label': label} for label in ['billing', 'technical', 'sales']]},
    {'id': 'frustration', 'type': 'score', 'instructions': 'How frustrated the customer appears', 'levels': ['Calm, just stating facts', 'Frustrated but civil', 'Very angry, strong language']},
]


@pytest.mark.parametrize('usage', [{}, {'input_tokens': None, 'output_tokens': None},
                                 {'input_tokens': 12}, {'output_tokens': 7}])
def test_optional_usage_counts_preserve_answers(server, monkeypatch, usage):
    monkeypatch.setitem(FIXTURE, 'usage', usage)
    base, _ = server
    result = decide(jev_decision('test-key', 'jev-latest', base + '/v1/systemone'),
                    {'message': 'Billed twice'}, QUESTIONS, max_retries=0)
    assert len(result['answers']) == 3
    assert result['usage']['input_tokens']['total'] == usage.get('input_tokens')
    assert result['usage']['output_tokens']['total'] == usage.get('output_tokens')
    assert result['usage']['raw'] == usage


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
    result = decide(model, {'message': 'Billed twice'}, QUESTIONS, max_retries=0, timeout={'total_ms': 2000})
    assert result['answers']['is_urgent'] == {'type': 'boolean', 'probability_true': 1.0}
    assert result['answers']['department']['selected'] == 'technical'
    assert result['answers']['frustration']['expected_value'] == 1.0
    assert result['answers']['frustration']['probabilities'] == [0, 1.0, 0]
    assert result['response']['body'] == FIXTURE
    assert result['probability_source'] == 'native'
    assert captures[0][0] == '/v1/systemone'
    assert captures[0][1]['authorization'] == 'Bearer test-key'
    assert captures[0][2]['questions']['is_urgent']['type'] == 'noul'


def test_invalid_answer_uses_python_error_class(server):
    base, _ = server
    model = jev_decision('test-key', 'jev-latest', base + '/invalid')
    with pytest.raises(APICallError):
        decide(model, 'text', QUESTIONS, max_retries=0, timeout={'total_ms': 2000})


@pytest.mark.parametrize('source', ['native', 'logit_scoring', 'model_estimate'])
def test_self_hosted_probability_source(server, source):
    base, _ = server
    model = jev_decision('test-key', 'jev-latest', base + '/v1/systemone', probability_source=source)
    result = decide(model, 'text', QUESTIONS, max_retries=0, timeout={'total_ms': 2000})
    assert result['probability_source'] == source


def test_unknown_probability_source():
    with pytest.raises(InvalidArgumentError):
        jev_decision('test-key', 'jev-latest', probability_source='unknown')


def test_official_structured_contract_and_capabilities():
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
        caps = decision_capabilities(model)
        assert caps['max_choices'] == 255
        assert caps['rounding'] == {'probability_decimals': 2, 'score_decimals': 2}
        assert not captures
        request = fixture['request']
        result = decide(model, request['state'], request['questions'], max_retries=0, timeout={'total_ms':2000})
        assert captures[0]['questions']['urgent']['criteria'] == request['questions'][0]['criteria']
        assert result['answers']['severity']['levels'] == request['questions'][2]['levels']
        assert result['rounding'] == caps['rounding']
    finally:
        http.shutdown()
        http.server_close()
        thread.join(timeout=2)


def test_openai_images_native_values_and_score_descriptions():
    from aimux import create_provider
    contract = json.loads((Path(__file__).resolve().parents[3] /
        'contract-tests/fixtures/decision-openai-full.json').read_text())
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
        assert result['answers']['choice']['value'] is True
        assert result['answers']['choice']['probabilities'] == {'boolean_true': 0.75, 'text_true': 0.25}
        assert result['answers']['score']['levels'] == contract['options']['questions'][1]['levels']
        assert decision_capabilities(model)['supports_images'] is True
    finally:
        http.shutdown()
        worker.join()
        http.server_close()
