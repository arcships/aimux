"""Blocking-entry inventory for the shared-ops Python API."""
from contextlib import contextmanager
import json
import multiprocessing
import os
from pathlib import Path
import tempfile
import time

from ops_fixtures import (
    OPENAI_RESPONSES, OPENAI_RESPONSES_STREAM, EMBEDDING_RESP, SPEECH_RESP, IMAGE_RESP,
    TRANSCRIPTION_RESP, RERANKING_RESP, SEARCH_RESP, FILES_RESP, slow_http, slow_ws,
)


@contextmanager
def model_at(url, method="languageModel", model_id="test-model", package="openai"):
    import aimux
    client = aimux.Client()
    with client.adopt(client.call("provider.create", {
        "package": package, "settings": {"apiKey": "test-key", "baseUrl": url},
    })["handle"]) as provider:
        result = provider.call("provider.files") if method == "files" else provider.call(
            "provider.model", {"method": method, "modelId": model_id},
        )
        with client.adopt(result if isinstance(result, str) else result["handle"]) as model:
            yield model


@contextmanager
def language(op):
    streaming = op in ("text.stream", "text.streamOpenAI", "text.consume")
    body = OPENAI_RESPONSES_STREAM if streaming else OPENAI_RESPONSES
    options = {"maxRetries": 0, "timeout": {"totalMs": 5000}}
    if op == "text.generateObject":
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
            if op.startswith("text.stream"):
                with model.stream(op, params) as events:
                    yield lambda: list(events)
            else:
                yield lambda: model.call(op, params)


MODALITIES = {
    "embedding": ("openai", "embeddingModel", "text-embedding-3-small", "embedding.embed", {"values": ["hello"]}, EMBEDDING_RESP, "application/json"),
    "embedding_many": ("openai", "embeddingModel", "text-embedding-3-small", "embedding.embedMany", {"values": ["hello"]}, EMBEDDING_RESP, "application/json"),
    "speech": ("openai", "speechModel", "tts-1", "speech.generate", {"text": "Hi", "voice": "alloy", "outputFormat": "mp3"}, SPEECH_RESP, "audio/mpeg"),
    "image": ("openai", "imageModel", "dall-e-3", "image.generate", {"prompt": "otter", "n": 1, "providerOptions": {}}, IMAGE_RESP, "application/json"),
    "transcription": ("openai", "transcriptionModel", "whisper-1", "transcription.generate", {"audio": "dGVzdA==", "mediaType": "audio/mp3"}, TRANSCRIPTION_RESP, "application/json"),
    "reranking": ("cohere", "rerankingModel", "rerank-v3.0", "reranking.rerank", {"query": "which?", "documents": {"type": "text", "values": ["a", "b"]}}, RERANKING_RESP, "application/json"),
    "search": ("tavily", "searchModel", "tavily", "search.search", {"query": "Rust?"}, SEARCH_RESP, "application/json"),
    "upload_file": ("openai", "files", "", "files.upload", {"data": {"type": "data", "data": "dGVzdA=="}, "mediaType": "application/pdf"}, FILES_RESP, "application/json"),
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
    # Core downloads this URL directly; the localhost provider fixture cannot
    # intercept it. Fake-IP DNS can make its non-public-address guard reject it.
    body = {
        "POST": json.dumps({"name": "operations/gil"}),
        "GET": json.dumps({"done": True, "response": {"generateVideoResponse": {
            "generatedSamples": [{"video": {"uri": "https://example.invalid/video.mp4"}}],
        }}}),
    }
    with slow_http(body) as url:
        with model_at(url, "videoModel", "veo-3.0", "google") as model:
            yield lambda: model.call("video.generate", {"prompt": "otter", "n": 1,
                "providerOptions": {}, "maxRetries": 0, "poll": {"intervalMs": 1, "timeoutMs": 5000}})


@contextmanager
def decision():
    fixture = Path(__file__).resolve().parents[3] / "aimux-providers/tests/fixtures/openai_decisions.json"
    contract = json.loads(fixture.read_text())
    with slow_http(json.dumps(contract["response"])) as url:
        with model_at(url, "decision", "gpt-4o") as model:
            yield lambda: model.call("decision.decide", {"options": contract["options"]})


@contextmanager
def discovery(catalogue=False):
    import aimux
    body = json.dumps({"providers": {}} if catalogue else {"data": [{"id": "gpt-4o", "object": "model"}]})
    with slow_http(body) as url:
        client = aimux.Client()
        if catalogue:
            yield lambda: client.call("catalogue.fetch", {"sourceUrl": url})
        else:
            with client.adopt(client.call("provider.create", {
                "package": "openai", "settings": {"apiKey": "test-key", "baseUrl": url},
            })["handle"]) as provider:
                yield lambda: provider.call("provider.invoke", {"method": "listModels"})


@contextmanager
def session():
    with slow_ws() as url:
        with model_at(url, "transcriptionModel", "gpt-realtime-whisper") as model:
            with model.client.adopt(model.call("transcription.start")) as operation:
                yield lambda: operation.call("operation.next", {"timeoutMs": 5000})


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
                    recorder = client.adopt(client.call("recorder.create", params)["handle"])
                    return recorder
                yield invoke
            else:
                opened.set()
                recorder = client.adopt(client.call("recorder.create", params)["handle"])
                recorder.call("recorder.attach")
                body = json.loads(OPENAI_RESPONSES)
                body["output"][0]["content"][0]["text"] = "x" * (2 * 1024 * 1024)
                with slow_http(json.dumps(body), delay=0) as url:
                    with model_at(url) as model:
                        model.call("text.generate", {"prompt": "hi", "options": {"maxRetries": 0}})
                if method == "close": recorder.call("recorder.stop")
                def invoke():
                    drain.set()
                    return recorder.close() if method == "close" else recorder.call("recorder.flush")
                yield invoke
        finally:
            opened.set()
            drain.set()
            if recorder is not None:
                recorder.call("recorder.stop")
                recorder.close()
            reader.join(timeout=2)
            if reader.is_alive(): reader.terminate(); reader.join(timeout=2)


ENTRY_POINTS = {
    "text_generate": ("generate", lambda: language("text.generate")),
    "text_object": ("generate", lambda: language("text.generateObject")),
    "text_openai": ("generate", lambda: language("text.generateOpenAI")),
    "text_consume": ("generate", lambda: language("text.consume")),
    "stream_next": ("poll", lambda: language("text.stream")),
    "stream_openai_next": ("poll", lambda: language("text.streamOpenAI")),
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
