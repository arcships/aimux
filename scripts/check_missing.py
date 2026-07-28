#!/usr/bin/env python3
"""Check which inventory providers still have no .rs implementation."""

import re
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
INV = REPO / "rfc" / "0004-provider-inventory.md"
SRC = REPO / "aimux-providers" / "src"

# Get all existing .rs module names
existing_mods = {f.stem for f in SRC.glob("*.rs") if f.stem != "lib"}

# Also check lib.rs for pub mod declarations
lib_rs = (SRC / "lib.rs").read_text(encoding="utf-8")
declared_mods = set(re.findall(r'^pub mod (\w+)', lib_rs, re.MULTILINE))

# Name mapping: inventory name -> possible .rs module names
# (inventory names that don't directly map to a filename)
NAME_ALIASES = {
    "ollama": ["ollama"],
    "openrouter": ["openrouter"],
    "mistralrs": ["mistralrs"],
    "llamafile": ["llamafile"],
    "doubleword": ["doubleword"],
    "copilot (github)": ["copilot"],
    "github_copilot": ["copilot"],
    "github (models)": ["github"],
    "chatgpt (订阅)": ["chatgpt"],
    "zai (智谱/z.ai)": ["zai"],
    "zhipu (智谱/glm)": ["bigmodel", "zai"],
    "baidu (文心/ernie)": ["baidu"],
    "tencent (混元/hunyuan)": ["tencent"],
    "xunfei (讯飞/spark)": ["xunfei"],
    "baichuan": ["baichuan"],
    "stepfun (阶跃)": ["stepfun"],
    "minimax": ["minimax"],
    "moonshot (kimi)": ["moonshotai"],
    "lingyiwanwu (零一/yi)": ["lingyiwanwu"],
    "yi (零一)": ["lingyiwanwu"],
    "360 (ai360/zhinao)": ["qihoo360"],
    "coze (扣子)": ["coze"],
    "siliconflow (硅基流动)": ["siliconflow"],
    "qiniu-ai (七牛)": ["qiniu_ai"],
    "longcat": ["longcat"],
    "bigmodel (智谱)": ["bigmodel"],
    "aihubmix": ["aihubmix"],
    "mira": ["mira"],
    "xiaomimimo (小米)": ["xiaomimimo"],
    "byteplus": ["byteplus"],
    "perfxcloud": ["perfxcloud"],
    "llama.cpp": ["llamacpp"],
    "lmstudio": ["lmstudio"],
    "vllm": ["vllm"],
    "sgl (sglang)": ["sglang"],
    "sglang": ["sglang"],
    "xinference": ["xinference"],
    "omlx / mlx_lm": ["omlx"],
    "mlx": ["mlx"],
    "local (通用本地)": ["local"],
    "cybertron": ["cybertron"],
    "jlama": ["jlama"],
    "localai": ["localai"],
    "onnx": ["onnx"],
    "openvino": ["openvino"],
    "gaudi": ["gaudi"],
    "jina": ["jina"],
    "jina_ai": ["jina"],
    "nomic": ["nomic"],
    "mixedbread": ["mixedbread"],
    "clip": ["clip"],
    "pg_vector": ["pg_vector"],
    "s3_vectors": ["s3_vectors"],
    "milvus (向量库)": ["milvus"],
    "qdrant (向量库)": ["qdrant"],
    "fastembed": ["fastembed"],
    "stability": ["stability_ai"],
    "stability-ai": ["stability_ai"],
    "recraft": ["recraft"],
    "runwayml": ["runwayml"],
    "runway": ["runwayml"],
    "aws_polly": ["aws_polly"],
    "nvidia_riva": ["nvidia_riva"],
    "soniox": ["soniox"],
    "midjourney": ["midjourney"],
    "sora": ["sora"],
    "vidu (视频)": ["vidu"],
    "vidu": ["vidu"],
    "jimeng (即梦)": ["jimeng"],
    "ideogram": ["ideogram"],
    "flux": ["flux"],
    "suno (音乐)": ["suno"],
    "suno": ["suno"],
    "meshy": ["meshy"],
    "tripo3d": ["tripo3d"],
    "segmind": ["segmind"],
    "runware": ["runware"],
    "sarvam": ["sarvam"],
    "murf": ["murf"],
    "playai": ["playai"],
    "speechify": ["speechify"],
    "inworld": ["inworld"],
    "ai21": ["ai21"],
    "anyscale": ["anyscale"],
    "sambanova": ["sambanova"],
    "predibase": ["predibase"],
    "triton (nvidia)": ["triton"],
    "triton": ["triton"],
    "databricks": ["databricks"],
    "sagemaker (aws)": ["sagemaker"],
    "watsonx (ibm)": ["watsonx"],
    "scaleway": ["scaleway"],
    "snowflake": ["snowflake"],
    "sap": ["sap"],
    "oci (oracle)": ["oci"],
    "nlp_cloud": ["nlp_cloud"],
    "friendliai": ["friendliai"],
    "clarifai": ["clarifai"],
    "gigachat (三星)": ["gigachat"],
    "codestral": ["codestral"],
    "morph": ["morph"],
    "v0": ["v0"],
    "aiml": ["aiml"],
    "heroku": ["heroku"],
    "hosted_vllm": ["hosted_vllm"],
    "nvidia_nim": ["nvidia_nim"],
    "nscale": ["nscale"],
    "lambda_ai": ["lambda_ai"],
    "petals": ["petals"],
    "oobabooba": ["oobabooba"],
    "inception": ["inception"],
    "galadriel": ["galadriel"],
    "gdc": ["gdc"],
    "datarobot": ["datarobot"],
    "infinity": ["infinity"],
    "kluster-ai": ["kluster_ai"],
    "krutrim": ["krutrim"],
    "bytez": ["bytez"],
    "upstage": ["upstage"],
    "deepbricks": ["deepbricks"],
    "lemonfox-ai": ["lemonfox_ai"],
    "inference-net": ["inference_net"],
    "302ai": ["ai302"],
    "matterai": ["matterai"],
    "nextbit": ["nextbit"],
    "modal": ["modal"],
    "aibadgr": ["aibadgr"],
    "ncompass": ["ncompass"],
    "reka-ai": ["reka_ai"],
    "sgl": ["sglang"],
    "parasail": ["parasail"],
    "wafer": ["wafer"],
    "bedrock_mantle": ["bedrock_mantle"],
    "nous (nousresearch)": ["nous_research"],
    "opencode-go": ["opencode_go"],
    "opencode-zen": ["opencode_zen"],
    "kiro": ["kiro"],
    "pioneer": ["pioneer"],
    "kilo": ["kilo"],
    "cline-pass": ["cline_pass"],
    "commandcode": ["commandcode"],
    "portkey": ["portkey"],
    "helicone": ["helicone"],
    "requesty": ["requesty"],
    "cometapi": ["cometapi"],
    "novita": ["novita"],
    "submodel": ["submodel"],
    "api2d": ["api2d"],
    "ohmygpt": ["ohmygpt"],
    "closeai": ["closeai"],
    "openaisb": ["openaisb"],
    "openaimax": ["openaimax"],
    "ails": ["ails"],
    "api2gpt": ["api2gpt"],
    "aigc2d": ["aigc2d"],
    "fastgpt": ["fastgpt"],
    "tokenpony": ["tokenpony"],
    "fastrouter": ["fastrouter"],
    "orcarouter": ["orcarouter"],
    "ovhcloud": ["ovhcloud"],
    "nebius": ["nebius"],
    "hyperbolic": ["hyperbolic"],
    "featherless-ai": ["featherless_ai"],
    "meta": ["meta_llama"],
    "meta_llama": ["meta_llama"],
    "modelscope (魔搭)": ["modelscope"],
    "docker_model_runner": ["docker_model_runner"],
    "ollama_cloud": ["ollama_cloud"],
    "litellm_proxy": ["litellm_proxy"],
    "compactifai": ["compactifai"],
    "doubaoaudio": ["doubaoaudio"],
    "mokaai": ["mokaai"],
    "skylark (云雀)": ["skylark"],
    "bing (new bing)": ["bing"],
    "slack (slack claude)": ["slack"],
    "deepl": ["deepl"],
    "dify": ["dify"],
    "doc2x": ["doc2x"],
    "sangforaicp": ["sangforaicp"],
    "streamlake": ["streamlake"],
    "antling": ["antling"],
    "text_embeddings_inference": ["text_embeddings_inference", "tei"],
    "tei (text embeddings inference)": ["tei", "text_embeddings_inference"],
    "sakana": ["sakana"],
    "scx-ai": ["scx_ai"],
    "atlascloud": ["atlascloud"],
    "canopywave": ["canopywave"],
    "embercloud": ["embercloud"],
    "tundra": ["tundra"],
    "reve": ["reve"],
    "gonka24": ["gonka24"],
    "albert": ["albert"],
    "fastcrw": ["fastcrw"],
    "apiserpent": ["apiserpent"],
    "nanogpt": ["nanogpt"],
    "vercel (ai gateway)": ["vercel"],
    "gradient_ai": ["gradient_ai"],
    "azure_ai": ["azure_ai"],
}

# Parse inventory for all provider names in tables
content = INV.read_text(encoding="utf-8")
cross = "\u274C"
check = "\u2705"

missing = []
for line in content.split("\n"):
    if not line.startswith("|"):
        continue
    parts = [p.strip() for p in line.split("|")]
    if len(parts) < 4:
        continue
    name = parts[1]
    if not name or name in ("厂商", "厂商/类型", "模块"):
        continue
    if name.startswith("-"):
        continue

    # Check if aimux column (parts[2]) still has ❌
    if cross not in parts[2]:
        continue  # already has some ✅

    # This provider is marked ❌ in aimux column
    # Check if we have an implementation
    aliases = NAME_ALIASES.get(name, [name.replace("-", "_").replace(".", "_").replace("/", "_").replace(" ", "_").lower()])
    # Also try the raw name
    aliases = list(set(aliases + [name.replace("-", "_").replace(".", "_").replace(" ", "_").lower()]))

    found = False
    for alias in aliases:
        if alias in existing_mods or alias in declared_mods:
            found = True
            break
    if not found:
        missing.append(name)

print(f"Still missing (cross mark in inventory, no .rs found): {len(missing)}")
for m in missing:
    print(f"  {m}")
