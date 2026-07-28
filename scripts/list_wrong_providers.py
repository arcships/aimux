#!/usr/bin/env python3
"""List providers to remove: NOT OpenAI Chat Completions compatible."""

# These were generated as thin wrappers but are NOT OpenAI-compatible LLM chat providers.
# They need different traits (ImageModel/VideoModel/SpeechModel/EmbeddingModel/etc.)
# or different auth (SigV4/IAM/OAuth) or are not LLM APIs at all.

REMOVE = {
    # Vector databases — not LLM APIs
    "milvus", "qdrant", "pg_vector", "s3_vectors",

    # Embedding/reranking only — not chat
    "clip", "fastembed", "tei", "text_embeddings_inference",
    "nomic", "jina", "mixedbread",

    # Image/Video/Music/3D generation — different API, not Chat Completions
    "recraft", "ideogram", "stability_ai", "segmind", "runware",
    "meshy", "tripo3d", "runwayml", "sora", "vidu",
    "jimeng", "midjourney", "flux", "suno",

    # Speech/TTS/STT — different API
    "murf", "playai", "speechify", "inworld",
    "aws_polly", "nvidia_riva", "soniox", "doubaoaudio", "mokaai",

    # Non-LLM services — not chat APIs
    "bing", "deepl", "dify", "slack", "doc2x",
    "streamlake", "antling", "sangforaicp", "skylark",

    # Special auth — not API key + Bearer
    "watsonx", "sagemaker", "sap", "oci", "snowflake", "bedrock_mantle",
}

print(f"Total to remove: {len(REMOVE)}")
for name in sorted(REMOVE):
    print(f"  {name}")
