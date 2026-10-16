"""Blocking-entry inventory for the shared-ops Python API."""
from contextlib import contextmanager
import json
import multiprocessing
import os
from pathlib import Path
import tempfile
import time

from aimux import ops
from ops_fixtures import (
    OPENAI_RESPONSES, OPENAI_RESPONSES_STREAM, EMBEDDING_RESP, SPEECH_RESP, IMAGE_RESP,
    TRANSCRIPTION_RESP, RERANKING_RESP, SEARCH_RESP, FILES_RESP, slow_http, slow_ws,
)


@contextmanager
def model_at(url, method="languageModel", model_id="test-model", package="openai"):
    import aimux
    client = aimux.Client()
    with client.adopt(client.call(ops.OP_PROVIDER_CREATE, {
        "package": package, "settings": {"apiKey": "test-key", "baseUrl": url},
    })["handle"]) as provider:
        result = provider.call(ops.OP_PROVIDER_FILES) if method == "files" else provider.call(
            ops.OP_PROVIDER_MODEL, {"method": method, "modelId": model_id},
        )
        with client.adopt(result if isinstance(result, str) else result["handle"]) as model:
            yield model


@contextmanager
def language(op):
    streaming = op in (ops.OP_TEXT_STREAM, ops.OP_TEXT_STREAM_OPEN_AI, ops.OP_TEXT_CONSUME)
    body = OPENAI_RESPONSES_STREAM if streaming else OPENAI_RESPONSES
    options = {"maxRetries": 0, "timeout": {"totalMs": 5000}}
    if op == ops.OP_TEXT_GENERATE_OBJECT:
        body = json.loads(OPENAI_RESPONSES)
        body["output"][0]["content"][0]["text"] = '{"answer":"hello"}'
        body = json.dumps(body)
        options["output"] = {"type": "object", "schema": {
            "type": "object", "properties": {"answer": {"type": "string"}},
            "required": ["answer"], "additionalProperties": False,
        }}
    with slow_http(body, "text/event-stream" if streaming else "application/json") as url:
        with model_at(url, model_id="gpt-4o") as model:
            params = {"prompt": "hi", "options": options}
            if op in (ops.OP_TEXT_STREAM, ops.OP_TEXT_STREAM_OPEN_AI):
                with model.stream(op, params) as events:
                    yield lambda: list(events)
            else:
                yield lambda: model.call(op, params)


MODALITIES = {
    "embedding": ("openai", "embeddingModel", "text-embedding-3-small", ops.OP_EMBEDDING_EMBED, {"values": ["hello"]}, EMBEDDING_RESP, "application/json"),
    "embedding_many": ("openai", "embeddingModel", "text-embedding-3-small", ops.OP_EMBEDDING_EMBED_MANY, {"values": ["hello"]}, EMBEDDING_RESP, "application/json"),
    "speech": ("openai", "speechModel", "tts-1", ops.OP_SPEECH_GENERATE, {"text": "Hi", "voice": "alloy", "outputFormat": "mp3"}, SPEECH_RESP, "audio/mpeg"),
    "image": ("openai", "imageModel", "dall-e-3", ops.OP_IMAGE_GENERATE, {"prompt": "otter", "n": 1, "providerOptions": {}}, IMAGE_RESP, "application/json"),
    "transcription": ("openai", "transcriptionModel", "whisper-1", ops.OP_TRANSCRIPTION_GENERATE, {"audio": "dGVzdA==", "mediaType": "audio/mp3"}, TRANSCRIPTION_RESP, "application/json"),
    "reranking": ("cohere", "rerankingModel", "rerank-v3.0", ops.OP_RERANKING_RERANK, {"query": "which?", "documents": {"type": "text", "values": ["a", "b"]}}, RERANKING_RESP, "application/json"),
    "search": ("tavily", "searchModel", "tavily", ops.OP_SEARCH_SEARCH, {"query": "Rust?"}, SEARCH_RESP, "application/json"),
    "upload_file": ("openai", "files", "", ops.OP_FILES_UPLOAD, {"data": {"type": "data", "data": "dGVzdA=="}, "mediaType": "application/pdf"}, FILES_RESP, "application/json"),
}


@contextmanager
def modality(name):
    package, method, model_id, op, params, body, content_type = MODALITIES[name]
    # files.upload has no retry/timeout controls in its canonical DTO.
    if method != "files": params = dict(params, maxRetries=0, timeout={"totalMs": 5000})
    with slow_http(body, content_type) as url:
        with model_at(url, method, model_id, package) as model:
            yield lambda: model.call(op, params)


@contextmanager
def video():
    # Inline bytes keep the result off core's download path, whose
    # non-public-address guard refuses the localhost fixture.
    body = {
        ":predictLongRunning": json.dumps({"name": "operations/gil"}),
        ":fetchPredictOperation": json.dumps({"done": True, "response": {"videos": [
            {"bytesBase64Encoded": "dGVzdA==", "mimeType": "video/mp4"},
        ]}}),
    }
    with slow_http(body) as url:
        with model_at(url, "videoModel", "veo-3.0-generate-001", "google_vertex") as model:
            yield lambda: model.call(ops.OP_VIDEO_GENERATE, {"prompt": "otter", "n": 1,
                "providerOptions": {}, "maxRetries": 0, "poll": {"intervalMs": 1, "timeoutMs": 5000}})


@contextmanager
def decision():
    fixture = Path(__file__).resolve().parents[3] / "aimux-providers/tests/fixtures/openai_decisions.json"
    contract = json.loads(fixture.read_text())
    with slow_http(json.dumps(contract["response"])) as url:
        with model_at(url, "decision", "gpt-4o") as model:
            yield lambda: model.call(ops.OP_DECISION_DECIDE, {"options": contract["options"]})


@contextmanager
def discovery(catalogue=False):
    import aimux
    if catalogue:
        import pytest
        # catalogue.fetch refuses loopback sources before any I/O, so no local
        # fixture can make it block; it shares Client.call with list_models.
        pytest.skip("catalogue.fetch refuses non-public sources; same native call path as list_models")
    body = json.dumps({"data": [{"id": "gpt-4o", "object": "model"}]})
    with slow_http(body) as url:
        client = aimux.Client()
        with client.adopt(client.call(ops.OP_PROVIDER_CREATE, {
            "package": "openai", "settings": {"apiKey": "test-key", "baseUrl": url},
        })["handle"]) as provider:
            yield lambda: provider.call(ops.OP_PROVIDER_INVOKE, {"method": "listModels"})


@contextmanager
def session():
    with slow_ws() as url:
        with model_at(url, "transcriptionModel", "gpt-realtime-whisper") as model:
            with model.client.adopt(model.call(ops.OP_TRANSCRIPTION_START)) as operation:
                yield lambda: operation.call(ops.OP_OPERATION_NEXT, {"timeoutMs": 5000})


def _fifo_reader(path, opened, drain):
    if not opened.wait(15): raise RuntimeError("FIFO open gate timed out")
    time.sleep(0.5)
    with open(path, "rb", buffering=0) as reader:
        if not drain.wait(15): raise RuntimeError("FIFO drain gate timed out")
        time.sleep(0.5)
        while reader.read(65536): pass


@contextmanager
def recording(method):
    import aimux
    if not hasattr(os, "mkfifo"):
        import pytest
        pytest.skip("slow recording I/O requires POSIX FIFOs")
    with tempfile.TemporaryDirectory(prefix="aimux-gil-") as directory:
        path = Path(directory) / "recordings.jsonl"
        os.mkfifo(path)
        opened, drain = multiprocessing.Event(), multiprocessing.Event()
        reader = multiprocessing.Process(target=_fifo_reader, args=(str(path), opened, drain))
        reader.start()
        client, recorder = aimux.Client(), None
        params = {"backend": {"type": "jsonl", "directory": directory}, "capturePolicy": "full"}
        try:
            if method == "create":
                def invoke():
                    nonlocal recorder
                    opened.set()
                    recorder = client.adopt(client.call(ops.OP_RECORDER_CREATE, params)["handle"])
                    return recorder
                yield invoke
            else:
                opened.set()
                recorder = client.adopt(client.call(ops.OP_RECORDER_CREATE, params)["handle"])
                recorder.call(ops.OP_RECORDER_ATTACH)
                body = json.loads(OPENAI_RESPONSES)
                body["output"][0]["content"][0]["text"] = "x" * (2 * 1024 * 1024)
                with slow_http(json.dumps(body), delay=0) as url:
                    with model_at(url) as model:
                        model.call(ops.OP_TEXT_GENERATE, {"prompt": "hi", "options": {"maxRetries": 0}})
                if method == "close": recorder.call(ops.OP_RECORDER_STOP)
                def invoke():
                    nonlocal recorder
                    drain.set()
                    if method != "close":
                        return recorder.call(ops.OP_RECORDER_FLUSH)
                    # A closed handle is gone; teardown must not stop it again.
                    closing, recorder = recorder, None
                    return closing.close()
                yield invoke
        finally:
            opened.set()
            drain.set()
            if recorder is not None:
                recorder.call(ops.OP_RECORDER_STOP)
                recorder.close()
            reader.join(timeout=2)
            if reader.is_alive(): reader.terminate(); reader.join(timeout=2)


ENTRY_POINTS = {
    "text_generate": ("generate", lambda: language(ops.OP_TEXT_GENERATE)),
    "text_object": ("generate", lambda: language(ops.OP_TEXT_GENERATE_OBJECT)),
    "text_openai": ("generate", lambda: language(ops.OP_TEXT_GENERATE_OPEN_AI)),
    "text_consume": ("generate", lambda: language(ops.OP_TEXT_CONSUME)),
    "stream_next": ("poll", lambda: language(ops.OP_TEXT_STREAM)),
    "stream_openai_next": ("poll", lambda: language(ops.OP_TEXT_STREAM_OPEN_AI)),
    **{name: ("generate", lambda name=name: modality(name)) for name in MODALITIES},
    "video": ("generate", video),
    "decision": ("generate", decision),
    "list_models": ("discovery", discovery),
    "catalogue": ("discovery", lambda: discovery(True)),
    "operation_next": ("poll", session),
    "recorder_create": ("construction_io", lambda: recording("create")),
    "recorder_flush": ("flush", lambda: recording("flush")),
    "recorder_close": ("join", lambda: recording("close")),
}
