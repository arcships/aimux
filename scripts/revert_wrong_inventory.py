#!/usr/bin/env python3
"""Revert wrong providers in inventory: ✅薄 → ❌ for providers that were removed."""

from pathlib import Path
import re

INV = Path(__file__).resolve().parent.parent / "rfc" / "0004-provider-inventory.md"
content = INV.read_text(encoding="utf-8")

check_thin = "\u2705\u8584"  # ✅薄
cross = "\u274C"  # ❌

# Provider names in inventory that should be reverted to ❌
# Format: (inventory_name_variants)
TO_REVERT = [
    # Vector databases
    "milvus (向量库)", "qdrant (向量库)", "pg_vector", "s3_vectors",
    # Embedding-only
    "clip", "fastembed", "tei (text embeddings inference)",
    "text_embeddings_inference", "nomic", "jina", "mixedbread",
    # Image/Video/Music/3D
    "recraft", "ideogram", "flux", "suno (音乐)", "suno",
    "midjourney", "sora", "vidu (视频)", "vidu", "jimeng (即梦)",
    "meshy", "tripo3d", "segmind", "runware", "runwayml", "runway",
    "stability", "stability-ai",
    # Speech/TTS/STT
    "murf", "playai", "speechify", "inworld",
    "aws_polly", "nvidia_riva", "soniox", "doubaoaudio", "mokaai",
    # Non-LLM
    "bing (new bing)", "deepl", "dify", "slack (slack claude)",
    "doc2x", "streamlake", "antling", "sangforaicp", "skylark (云雀)",
    # Special auth
    "watsonx (ibm)", "sagemaker (aws)", "sap",
    "oci (oracle)", "snowflake", "bedrock_mantle",
]

lines = content.split("\n")
changed = 0
new_lines = []
for line in lines:
    if line.startswith("|") and check_thin in line:
        # Check if this line matches any of the names to revert
        parts = [p.strip() for p in line.split("|")]
        if len(parts) >= 3:
            name = parts[1]
            for revert_name in TO_REVERT:
                if name == revert_name:
                    # Replace first ✅薄 with ❌
                    line = line.replace(check_thin, cross, 1)
                    changed += 1
                    break
    new_lines.append(line)

INV.write_text("\n".join(new_lines), encoding="utf-8")
print(f"Reverted {changed} lines")
