"""Decision contract crosses Python -> PyO3 -> core -> HTTP -> Python dicts.

A thread-based server also verifies the native decision call releases the GIL.
"""
import json
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

import pytest
from aimux import APICallError, InvalidArgumentError, decide, jev_decision, decision_capabilities

FIXTURE = json.loads((Path(__file__).resolve().parents[3] / 'aimux-providers/tests/fixtures/jev_systemone.json').read_text())['response']
QUESTIONS = [
    {'id': 'is_urgent', 'type': 'boolean', 'instructions': 'The message conveys urgency or time-sensitivity'},
    {'id': 'department', 'type': 'choice', 'instructions': 'Which team should handle this', 'options': [{'label': label} for label in ['billing', 'technical', 'sales']]},
    {'id': 'frustration', 'type': 'score', 'instructions': 'How frustrated the customer appears', 'levels': ['Calm, just stating facts', 'Frustrated but civil', 'Very angry, strong language']},
]


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
