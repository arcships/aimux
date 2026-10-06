"""Decision contract crosses Python -> PyO3 -> core -> HTTP -> Python dicts.

A thread-based server also verifies the native decision call releases the GIL.
"""
import json
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

import pytest
from aimux import APICallError, decide, jev_decision

FIXTURE = json.loads((Path(__file__).resolve().parents[3] / 'aimux-providers/tests/fixtures/jev_systemone.json').read_text())['response']
QUESTIONS = [
    {'id': 'needs_human', 'type': 'boolean', 'instructions': 'Does this need a human?'},
    {'id': 'queue', 'type': 'choice', 'instructions': 'Which team?', 'options': [{'label': label} for label in ['billing', 'technical', 'sales']]},
    {'id': 'anger', 'type': 'score', 'instructions': 'How angry?', 'levels': ['Calm', 'Mildly annoyed', 'Frustrated', 'Angry']},
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
    model = jev_decision('test-key', 'jev-1.13', base + '/v1/systemone')
    result = decide(model, {'message': 'Billed twice'}, QUESTIONS, max_retries=0, timeout={'total_ms': 2000})
    assert result['answers']['needs_human'] == {'type': 'boolean', 'probability_true': 0.89}
    assert result['answers']['queue']['selected'] == 'billing'
    assert result['answers']['anger']['expected_value'] == 1.89
    assert result['answers']['anger']['probabilities'] == [0, 0.11, 0.89, 0]
    assert result['response']['body'] == FIXTURE
    assert result['probability_source'] == 'native'
    assert captures[0][0] == '/v1/systemone'
    assert captures[0][1]['authorization'] == 'Bearer test-key'
    assert captures[0][2]['questions']['needs_human']['type'] == 'noul'


def test_invalid_answer_uses_python_error_class(server):
    base, _ = server
    model = jev_decision('test-key', 'jev-1.13', base + '/invalid')
    with pytest.raises(APICallError):
        decide(model, 'text', QUESTIONS, max_retries=0, timeout={'total_ms': 2000})
