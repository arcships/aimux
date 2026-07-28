#!/usr/bin/env python3
"""Generate thin-wrapper provider .rs files for ALL missing OpenAI-compatible providers.

This is a one-shot bulk generation script. It creates a .rs file per provider,
updates lib.rs, and prints a summary.

Usage:
    uv run python scripts/generate_all_providers.py
"""

import os
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
SRC = REPO / "aimux-providers" / "src"

# ── Already implemented (don't regenerate) ──
ALREADY_DONE = {
    "anthropic", "anthropic_aws", "azure", "bedrock", "google", "vertex",
    "mistral", "cohere", "xai", "deepseek", "openai", "open_responses",
    # OpenAI-compatible thin wrappers (batch 1)
    "groq", "fireworks", "togetherai", "perplexity", "moonshotai", "cerebras",
    "alibaba", "baseten", "bytedance", "deepinfra", "huggingface", "vercel",
    "openrouter", "copilot", "llamafile", "mistralrs", "doubleword",
    # Non-text
    "voyage", "cartesia", "elevenlabs", "hume", "lmnt",
    "assemblyai", "deepgram", "fal", "gladia", "revai",
    "black_forest_labs", "luma", "prodia", "replicate", "klingai",
    # Batch 2 (just added)
    "ollama", "zai", "github", "siliconflow", "lmstudio", "sambanova",
}

# ── Provider definitions ──
# (module_name, struct_prefix, base_url, env_var, is_local, display_name, doc_url)
# is_local=True: no API key needed, uses ENV_VAR for base URL
# is_local=False: uses ENV_VAR for API key
PROVIDERS = [
    # ── Cloud LLM providers (OpenAI-compatible) ──
    ("novita", "Novita", "https://api.novita.ai/v1", "NOVITA_API_KEY", False, "Novita AI", "novita.ai"),
    ("nebius", "Nebius", "https://api.studio.nebius.ai/v1", "NEBIUS_API_KEY", False, "Nebius AI", "nebius.ai"),
    ("hyperbolic", "Hyperbolic", "https://api.hyperbolic.xyz/v1", "HYPERBOLIC_API_KEY", False, "Hyperbolic", "hyperbolic.xyz"),
    ("ovhcloud", "Ovhcloud", "https://oai.endpoints.kepler.ai.cloud.ovh.net/v1", "OVHCLOUD_API_KEY", False, "OVHcloud AI", "ovhcloud.com"),
    ("ai21", "Ai21", "https://api.ai21.ai/v1", "AI21_API_KEY", False, "AI21 Labs", "ai21.com"),
    ("anyscale", "Anyscale", "https://api.endpoints.anyscale.com/v1", "ANYSCALE_API_KEY", False, "Anyscale", "anyscale.com"),
    ("predibase", "Predibase", "https://serving.app.predibase.com/v1", "PREDIBASE_API_KEY", False, "Predibase", "predibase.com"),
    ("databricks", "Databricks", "https://databricks.com/serving-endpoints", "DATABRICKS_API_KEY", False, "Databricks", "databricks.com"),
    ("scaleway", "Scaleway", "https://api.scaleway.ai/v1", "SCALEWAY_API_KEY", False, "Scaleway AI", "scaleway.com"),
    ("nlp_cloud", "NlpCloud", "https://api.nlpcloud.io/v1", "NLPCLOUD_API_KEY", False, "NLP Cloud", "nlpcloud.com"),
    ("friendliai", "FriendliAI", "https://inference.friendli.ai/v1", "FRIENDLIAI_API_KEY", False, "FriendliAI", "friendli.ai"),
    ("clarifai", "Clarifai", "https://api.clarifai.com/v2/ext/openai/v1", "CLARIFAI_API_KEY", False, "Clarifai", "clarifai.com"),
    ("gigachat", "GigaChat", "https://gigachat.devices.sberbank.ru/api/v1", "GIGACHAT_API_KEY", False, "GigaChat (Sberbank)", "sberbank.ru"),
    ("codestral", "Codestral", "https://api.mistral.ai/v1", "CODESTRAL_API_KEY", False, "Codestral (Mistral)", "mistral.ai"),
    ("morph", "Morph", "https://api.morphllm.com/v1", "MORPH_API_KEY", False, "Morph LLM", "morphllm.com"),
    ("aiml", "Aiml", "https://api.aimlapi.com/v1", "AIML_API_KEY", False, "AI/ML API", "aimlapi.com"),
    ("heroku", "Heroku", "https://api.heroku.com/inference/v1", "HEROKU_API_KEY", False, "Heroku AI", "heroku.com"),
    ("nvidia_nim", "NvidiaNim", "https://integrate.api.nvidia.com/v1", "NVIDIA_API_KEY", False, "NVIDIA NIM", "nvidia.com"),
    ("nscale", "Nscale", "https://inference.api.nscale.com/v1", "NSCALE_API_KEY", False, "Nscale", "nscale.com"),
    ("lambda_ai", "LambdaAi", "https://api.lambda.ai/v1", "LAMBDA_API_KEY", False, "Lambda AI", "lambda.ai"),
    ("inception", "Inception", "https://api.inceptionlabs.ai/v1", "INCEPTION_API_KEY", False, "Inception Labs", "inceptionlabs.ai"),
    ("galadriel", "Galadriel", "https://api.galadriel.com/v1", "GALADRIEL_API_KEY", False, "Galadriel", "galadriel.com"),
    ("datarobot", "Datarobot", "https://app.datarobot.com/api/v2", "DATAROBOT_API_TOKEN", False, "DataRobot", "datarobot.com"),
    ("featherless_ai", "FeatherlessAi", "https://api.featherless.ai/v1", "FEATHERLESS_API_KEY", False, "Featherless AI", "featherless.ai"),
    ("cometapi", "Cometapi", "https://api.cometapi.com/v1", "COMETAPI_API_KEY", False, "CometAPI", "cometapi.com"),
    ("reka_ai", "RekaAi", "https://api.reka.ai/v1", "REKA_API_KEY", False, "Reka AI", "reka.ai"),
    ("sarvam", "Sarvam", "https://api.sarvam.ai/v1", "SARVAM_API_KEY", False, "Sarvam AI", "sarvam.ai"),
    ("meta_llama", "MetaLlama", "https://api.llama.com/compat/v1", "LLAMA_API_KEY", False, "Meta Llama API", "llama.com"),
    ("apiserpent", "Apiserpent", "https://api.apiserpent.com/v1", "APISERPENT_API_KEY", False, "API Serpent", "apiserpent.com"),
    ("modelscope", "Modelscope", "https://api-inference.modelscope.cn/v1", "MODELSCOPE_API_KEY", False, "ModelScope", "modelscope.cn"),
    ("litellm_proxy", "LitellmProxy", "http://127.0.0.1:4000/v1", "LITELLM_PROXY_API_KEY", True, "LiteLLM Proxy", "litellm.ai"),
    ("compactifai", "Compactifai", "https://api.compactif.ai/v1", "COMPACTIFAI_API_KEY", False, "CompactifAI", "compactif.ai"),
    ("gradient_ai", "GradientAi", "https://inference.do-ai.run/v1", "GRADIENT_API_KEY", False, "Gradient AI", "do-ai.run"),
    ("azure_ai", "AzureAi", "https://models.inference.ai.azure.com", "AZURE_AI_API_KEY", False, "Azure AI", "azure.com"),
    ("doubaoaudio", "DoubaoAudio", "https://ark.cn-beijing.volces.com/api/v3", "ARK_API_KEY", False, "Doubao Audio", "volces.com"),

    # ── 国产厂商 (OpenAI-compatible) ──
    ("baidu", "Baidu", "https://qianfan.baidubce.com/v2", "BAIDU_API_KEY", False, "Baidu (文心/ERNIE)", "baidubce.com"),
    ("tencent", "Tencent", "https://api.hunyuan.cloud.tencent.com/v1", "TENCENT_API_KEY", False, "Tencent (混元/Hunyuan)", "tencent.com"),
    ("baichuan", "Baichuan", "https://api.baichuan-ai.com/v1", "BAICHUAN_API_KEY", False, "Baichuan AI", "baichuan-ai.com"),
    ("stepfun", "Stepfun", "https://api.stepfun.com/v1", "STEPFUN_API_KEY", False, "StepFun (阶跃星辰)", "stepfun.com"),
    ("minimax", "Minimax", "https://api.minimax.io/v1", "MINIMAX_API_KEY", False, "MiniMax", "minimax.io"),
    ("lingyiwanwu", "Lingyiwanwu", "https://api.lingyiwanwu.com/v1", "LINGYIWANWU_API_KEY", False, "Lingyiwanwu (零一万物)", "lingyiwanwu.com"),
    ("qihoo360", "Qihoo360", "https://api.360.cn/v1", "AI360_API_KEY", False, "360 AI", "360.cn"),
    ("coze", "Coze", "https://api.coze.cn/v1", "COZE_API_KEY", False, "Coze (扣子)", "coze.cn"),
    ("qiniu_ai", "QiniuAi", "https://api.qiniu.com/v1", "QINIU_API_KEY", False, "Qiniu AI", "qiniu.com"),
    ("longcat", "Longcat", "https://api.longcat.chat/v1", "LONGCAT_API_KEY", False, "LongCat", "longcat.chat"),
    ("bigmodel", "Bigmodel", "https://open.bigmodel.cn/api/paas/v4", "BIGMODEL_API_KEY", False, "BigModel (智谱)", "bigmodel.cn"),
    ("aihubmix", "Aihubmix", "https://aihubmix.com/v1", "AIHUBMIX_API_KEY", False, "AIHubMix", "aihubmix.com"),
    ("mira", "Mira", "https://api.mira.so/v1", "MIRA_API_KEY", False, "Mira", "mira.so"),
    ("xiaomimimo", "Xiaomimimo", "https://mimo.xiaomi.com/v1", "XIAOMI_API_KEY", False, "Xiaomi MiMo", "xiaomi.com"),
    ("byteplus", "Byteplus", "https://ark.bytepluses.com/api/v3", "BYTEPLUS_API_KEY", False, "BytePlus (Volcano)", "bytepluses.com"),
    ("perfxcloud", "Perfxcloud", "https://api.perfxcloud.com/v1", "PERFXCLOUD_API_KEY", False, "PerfXCloud", "perfxcloud.com"),

    # ── 本地推理 (OpenAI-compatible endpoints) ──
    ("llamacpp", "Llamacpp", "http://127.0.0.1:8080/v1", "LLAMACPP_BASE_URL", True, "llama.cpp", "github.com/ggerganov/llama.cpp"),
    ("vllm", "Vllm", "http://127.0.0.1:8000/v1", "VLLM_BASE_URL", True, "vLLM", "vllm.ai"),
    ("sglang", "Sglang", "http://127.0.0.1:30000/v1", "SGLANG_BASE_URL", True, "SGLang", "github.com/sgl-project/sglang"),
    ("xinference", "Xinference", "http://127.0.0.1:9997/v1", "XINFERENCE_BASE_URL", True, "Xinference", "inference.ai"),
    ("localai", "Localai", "http://127.0.0.1:8080/v1", "LOCALAI_BASE_URL", True, "LocalAI", "localai.io"),
    ("jlama", "Jlama", "http://127.0.0.1:8080/v1", "JLAMA_BASE_URL", True, "Jlama", "github.com/tjake/Jlama"),
    ("ollama_cloud", "OllamaCloud", "https://api.ollama.com/v1", "OLLAMA_CLOUD_API_KEY", False, "Ollama Cloud", "ollama.com"),
    ("docker_model_runner", "DockerModelRunner", "http://model-runner.docker.internal/engines/llama.cpp/v1", "DOCKER_MODEL_RUNNER_BASE_URL", True, "Docker Model Runner", "docker.com"),

    # ── 网关/聚合 ──
    ("portkey", "Portkey", "https://api.portkey.ai/v1", "PORTKEY_API_KEY", False, "Portkey Gateway", "portkey.ai"),
    ("helicone", "Helicone", "https://api.helicone.ai/v1", "HELICONE_API_KEY", False, "Helicone", "helicone.ai"),
    ("requesty", "Requesty", "https://api.requesty.ai/v1", "REQUESTY_API_KEY", False, "Requesty", "requesty.ai"),
    ("ai302", "Ai302", "https://api.302.ai/v1", "AI302_API_KEY", False, "302.AI", "302.ai"),
    ("api2d", "Api2d", "https://oa.api2d.net/v1", "API2D_API_KEY", False, "API2D", "api2d.net"),
    ("ohmygpt", "Ohmygpt", "https://api.ohmygpt.com/v1", "OHMYGPT_API_KEY", False, "OhMyGPT", "ohmygpt.com"),
    ("closeai", "Closeai", "https://api.closeai-proxy.xyz/v1", "CLOSEAI_API_KEY", False, "CloseAI", "closeai-proxy.xyz"),
    ("openaisb", "Openaisb", "https://api.openaisb.com/v1", "OPENAISB_API_KEY", False, "OpenAI-SB", "openaisb.com"),
    ("openaimax", "Openaimax", "https://api.openaimax.com/v1", "OPENAIMAX_API_KEY", False, "OpenAIMax", "openaimax.com"),
    ("ails", "Ails", "https://api.caipacity.com/v1", "AILS_API_KEY", False, "AILS", "caipacity.com"),
    ("api2gpt", "Api2gpt", "https://api.api2gpt.com/v1", "API2GPT_API_KEY", False, "API2GPT", "api2gpt.com"),
    ("aigc2d", "Aigc2d", "https://api.aigc2d.com/v1", "AIGC2D_API_KEY", False, "AIGC2D", "aigc2d.com"),
    ("fastgpt", "Fastgpt", "https://api.fastgpt.in/v1", "FASTGPT_API_KEY", False, "FastGPT", "fastgpt.in"),
    ("tokenpony", "Tokenpony", "https://api.tokenpony.com/v1", "TOKENPONY_API_KEY", False, "TokenPony", "tokenpony.com"),
    ("fastrouter", "Fastrouter", "https://api.fastrouter.ai/v1", "FASTROUTER_API_KEY", False, "FastRouter", "fastrouter.ai"),
    ("orcarouter", "Orcarouter", "https://api.orcarouter.com/v1", "ORCAROUTER_API_KEY", False, "OrcaRouter", "orcarouter.com"),
    ("submodel", "Submodel", "https://api.submodel.com/v1", "SUBMODEL_API_KEY", False, "SubModel", "submodel.com"),

    # ── 其他云厂商 ──
    ("kluster_ai", "KlusterAi", "https://api.kluster.ai/v1", "KLUSTER_API_KEY", False, "Kluster AI", "kluster.ai"),
    ("krutrim", "Krutrim", "https://api.krutrim.ai/v1", "KRUTRIM_API_KEY", False, "Krutrim", "krutrim.ai"),
    ("bytez", "Bytez", "https://api.bytez.com/v2", "BYTEZ_API_KEY", False, "Bytez", "bytez.com"),
    ("upstage", "Upstage", "https://api.upstage.ai/v1", "UPSTAGE_API_KEY", False, "Upstage", "upstage.ai"),
    ("deepbricks", "Deepbricks", "https://api.deepbricks.ai/v1", "DEEPBRICKS_API_KEY", False, "DeepBricks", "deepbricks.ai"),
    ("lemonfox_ai", "LemonfoxAi", "https://api.lemonfox.ai/v1", "LEMONFOX_API_KEY", False, "Lemonfox AI", "lemonfox.ai"),
    ("modal", "Modal", "https://modal.com/v1", "MODAL_API_KEY", False, "Modal", "modal.com"),
    ("sakana", "Sakana", "https://api.sakana.ai/v1", "SAKANA_API_KEY", False, "Sakana AI", "sakana.ai"),
    ("nous_research", "NousResearch", "https://api.nousresearch.com/v1", "NOUS_API_KEY", False, "Nous Research", "nousresearch.com"),
    ("bedrock_mantle", "BedrockMantle", "https://bedrock-runtime.us-east-1.amazonaws.com", "AWS_BEDROCK_API_KEY", False, "Bedrock Mantle", "aws.amazon.com"),
    ("watsonx", "Watsonx", "https://us-south.ml.cloud.ibm.com/v1", "WATSONX_API_KEY", False, "IBM watsonx", "ibm.com"),
    ("sagemaker", "Sagemaker", "https://runtime.sagemaker.us-east-1.amazonaws.com", "SAGEMAKER_API_KEY", False, "AWS SageMaker", "aws.amazon.com"),
    ("sap", "Sap", "https://api.ai.sap.eu10.hana.ondemand.com/v2", "SAP_AI_API_KEY", False, "SAP AI Core", "sap.com"),
    ("oci", "Oci", "https://inference.generativeai.us-chicago-1.oci.oraclecloud.com", "OCI_API_KEY", False, "Oracle OCI AI", "oracle.com"),
    ("snowflake", "Snowflake", "https://xxx.snowflakecomputing.com/api/v2/cortex", "SNOWFLAKE_API_KEY", False, "Snowflake Cortex", "snowflake.com"),
    ("infinity", "Infinity", "https://infinity.ai/api/v1", "INFINITY_API_KEY", False, "Infinity AI", "infinity.ai"),
    ("hosted_vllm", "HostedVllm", "https://hosted-vllm-api.com/v1", "HOSTED_VLLM_API_KEY", False, "Hosted vLLM", "vllm.ai"),
    ("petals", "Petals", "https://api.petals.dev/v1", "PETALS_API_KEY", False, "Petals", "petals.dev"),
    ("oobabooba", "Oobabooba", "http://127.0.0.1:5000/v1", "OOBABOOBA_BASE_URL", True, "Oobabooga Text Generation WebUI", "github.com/oobabooga"),
    ("gdc", "Gdc", "https://api.gdc.ai/v1", "GDC_API_KEY", False, "GDC", "gdc.ai"),
    ("fastcrw", "Fastcrw", "https://fastcrw.com/api/v1", "FASTCRW_API_KEY", False, "FastCRW", "fastcrw.com"),
    ("dify", "Dify", "https://api.dify.ai/v1", "DIFY_API_KEY", False, "Dify", "dify.ai"),
    ("clip", "Clip", "http://127.0.0.1:8080/v1", "CLIP_BASE_URL", True, "CLIP (local)", "openai.com/clip"),
    ("fastembed", "Fastembed", "http://127.0.0.1:8080/v1", "FASTEMBED_BASE_URL", True, "FastEmbed (local)", "github.com/Anush008/fastembed-rs"),
    ("tei", "Tei", "http://127.0.0.1:8080/v1", "TEI_BASE_URL", True, "Text Embeddings Inference (local)", "github.com/huggingface/text-embeddings-inference"),
    ("nomic", "Nomic", "https://api.nomic.ai/v1", "NOMIC_API_KEY", False, "Nomic", "nomic.ai"),
    ("jina", "Jina", "https://api.jina.ai/v1", "JINA_API_KEY", False, "Jina AI", "jina.ai"),
    ("mixedbread", "Mixedbread", "https://api.mixedbread.ai/v1", "MIXEDBREAD_API_KEY", False, "Mixedbread", "mixedbread.ai"),
    ("recraft", "Recraft", "https://external.api.recraft.ai/v1", "RECRAFT_API_KEY", False, "Recraft", "recraft.ai"),
    ("ideogram", "Ideogram", "https://api.ideogram.ai/v1", "IDEOGRAM_API_KEY", False, "Ideogram", "ideogram.ai"),
    ("stability_ai", "StabilityAi", "https://api.stability.ai/v1", "STABILITY_API_KEY", False, "Stability AI", "stability.ai"),
    ("segmind", "Segmind", "https://api.segmind.com/v1", "SEGMIND_API_KEY", False, "Segmind", "segmind.com"),
    ("runware", "Runware", "https://api.runware.ai/v1", "RUNWARE_API_KEY", False, "Runware", "runware.ai"),
    ("meshy", "Meshy", "https://api.meshy.ai/v1", "MESHY_API_KEY", False, "Meshy (3D)", "meshy.ai"),
    ("tripo3d", "Tripo3d", "https://api.tripo3d.ai/v1", "TRIPO3D_API_KEY", False, "Tripo3D", "tripo3d.ai"),
    ("runwayml", "Runwayml", "https://api.dev.runwayml.com/v1", "RUNWAYML_API_KEY", False, "Runway ML", "runwayml.com"),
    ("sora", "Sora", "https://api.openai.com/v1", "OPENAI_API_KEY", False, "OpenAI Sora", "openai.com"),
    ("vidu", "Vidu", "https://api.vidu.com/v1", "VIDU_API_KEY", False, "Vidu", "vidu.com"),
    ("jimeng", "Jimeng", "https://api.jimeng.jianying.com/v1", "JIMENG_API_KEY", False, "Jimeng (即梦)", "jianying.com"),
    ("midjourney", "Midjourney", "https://api.midjourney.com/v1", "MIDJOURNEY_API_KEY", False, "Midjourney", "midjourney.com"),
    ("flux", "Flux", "https://api.fal.ai/v1", "FLUX_API_KEY", False, "FLUX (via fal)", "fal.ai"),
    ("suno", "Suno", "https://api.suno.com/v1", "SUNO_API_KEY", False, "Suno (音乐)", "suno.com"),
    ("doubaoaudio", "DoubaoAudio", "https://ark.cn-beijing.volces.com/api/v3", "ARK_API_KEY", False, "Doubao Audio", "volces.com"),
    ("murf", "Murf", "https://api.murf.ai/v1", "MURF_API_KEY", False, "Murf (TTS)", "murf.ai"),
    ("playai", "Playai", "https://api.play.ai/v1", "PLAYAI_API_KEY", False, "PlayAI (TTS)", "play.ai"),
    ("speechify", "Speechify", "https://api.sws.speechify.com/v1", "SPEECHIFY_API_KEY", False, "Speechify (TTS)", "speechify.com"),
    ("inworld", "Inworld", "https://api.inworld.ai/v1", "INWORLD_API_KEY", False, "Inworld (TTS)", "inworld.ai"),
    ("aws_polly", "AwsPolly", "https://polly.us-east-1.amazonaws.com", "AWS_POLLY_API_KEY", False, "AWS Polly (TTS)", "aws.amazon.com"),
    ("nvidia_riva", "NvidiaRiva", "https://api.nvidia.com/v1", "NVIDIA_RIVA_API_KEY", False, "NVIDIA Riva (TTS)", "nvidia.com"),
    ("soniox", "Soniox", "https://api.soniox.com/v1", "SONIOX_API_KEY", False, "Soniox (STT)", "soniox.com"),
    ("mokaai", "Mokaai", "https://api.mokaai.com/v1", "MOKAAI_API_KEY", False, "MokaAI", "mokaai.com"),
    ("skylark", "Skylark", "https://api.skylark.com/v1", "SKYLARK_API_KEY", False, "Skylark (云雀)", "skylark.com"),
    ("deepl", "Deepl", "https://api.deepl.com/v2", "DEEPL_API_KEY", False, "DeepL", "deepl.com"),
    ("bing", "Bing", "https://api.bing.microsoft.com/v1", "BING_API_KEY", False, "Bing", "microsoft.com"),
    ("slack", "Slack", "https://slack.com/api/chat.postMessage", "SLACK_API_KEY", False, "Slack (Claude proxy)", "slack.com"),
    ("doubaoaudio", "DoubaoAudio", "https://ark.cn-beijing.volces.com/api/v3", "ARK_API_KEY", False, "Doubao Audio", "volces.com"),
    ("ncompass", "Ncompass", "https://api.ncompass.tech/v1", "NCOMPASS_API_KEY", False, "Ncompass", "ncompass.tech"),
    ("parasail", "Parasail", "https://api.parasail.io/v1", "PARASAIL_API_KEY", False, "Parasail", "parasail.io"),
    ("wafer", "Wafer", "https://api.wafer.ai/v1", "WAFER_API_KEY", False, "Wafer", "wafer.ai"),
    ("matterai", "Matterai", "https://api.matterai.com/v1", "MATTERAI_API_KEY", False, "Matter AI", "matterai.com"),
    ("nextbit", "Nextbit", "https://api.nextbit.ai/v1", "NEXTBIT_API_KEY", False, "NextBit", "nextbit.ai"),
    ("aibadgr", "Aibadgr", "https://api.aibadgr.com/v1", "AIBADGR_API_KEY", False, "AI Badgr", "aibadgr.com"),
    ("inference_net", "InferenceNet", "https://api.inference.net/v1", "INFERENCE_NET_API_KEY", False, "Inference.net", "inference.net"),
    ("lemonfox_ai", "LemonfoxAi", "https://api.lemonfox.ai/v1", "LEMONFOX_API_KEY", False, "Lemonfox AI", "lemonfox.ai"),
    ("opencode_go", "OpencodeGo", "https://api.opencode.dev/v1", "OPENCODE_GO_API_KEY", False, "OpenCode Go", "opencode.dev"),
    ("opencode_zen", "OpencodeZen", "https://api.opencode.zen/v1", "OPENCODE_ZEN_API_KEY", False, "OpenCode Zen", "opencode.zen"),
    ("kiro", "Kiro", "https://api.kiro.dev/v1", "KIRO_API_KEY", False, "Kiro", "kiro.dev"),
    ("pioneer", "Pioneer", "https://api.pioneer.ai/v1", "PIONEER_API_KEY", False, "Pioneer", "pioneer.ai"),
    ("kilo", "Kilo", "https://api.kilo.ai/v1", "KILO_API_KEY", False, "Kilo", "kilo.ai"),
    ("commandcode", "Commandcode", "https://api.commandcode.com/v1", "COMMANDCODE_API_KEY", False, "CommandCode", "commandcode.com"),
    ("cline_pass", "ClinePass", "https://api.cline.bot/v1", "CLINE_API_KEY", False, "Cline", "cline.bot"),
    ("albert", "Albert", "https://api.albert.ai/v1", "ALBERT_API_KEY", False, "Albert", "albert.ai"),
    ("scx_ai", "ScxAi", "https://api.scx.ai/v1", "SCX_AI_API_KEY", False, "SCX AI", "scx.ai"),
    ("atlascloud", "Atlascloud", "https://api.atlascloud.com/v1", "ATLASCLOUD_API_KEY", False, "AtlasCloud", "atlascloud.com"),
    ("canopywave", "Canopywave", "https://api.canopywave.com/v1", "CANOPYWAVE_API_KEY", False, "Canopywave", "canopywave.com"),
    ("embercloud", "Embercloud", "https://api.embercloud.com/v1", "EMBERCLOUD_API_KEY", False, "Embercloud", "embercloud.com"),
    ("tundra", "Tundra", "https://api.tundra.ai/v1", "TUNDRA_API_KEY", False, "Tundra", "tundra.ai"),
    ("reve", "Reve", "https://api.reve.ai/v1", "REVE_API_KEY", False, "Reve", "reve.ai"),
    ("gonka24", "Gonka24", "https://api.gonka24.com/v1", "GONKA24_API_KEY", False, "Gonka24", "gonka24.com"),
    ("streamlake", "Streamlake", "https://api.streamlake.com/v1", "STREAMLAKE_API_KEY", False, "StreamLake", "streamlake.com"),
    ("antling", "Antling", "https://api.antling.com/v1", "ANTLING_API_KEY", False, "Antling", "antling.com"),
    ("sangforaicp", "Sangforaicp", "https://aicp.sangfor.com/v1", "SANGFOR_AICP_API_KEY", False, "Sangfor AICP", "sangfor.com"),
    ("doc2x", "Doc2x", "https://api.doc2x.com/v1", "DOC2X_API_KEY", False, "Doc2X", "doc2x.com"),
    ("v0", "V0", "https://api.v0.dev/v1", "V0_API_KEY", False, "v0 (Vercel)", "v0.dev"),
    ("text_embeddings_inference", "TextEmbeddingsInference", "http://127.0.0.1:8080", "TEI_BASE_URL", True, "Text Embeddings Inference", "github.com/huggingface/text-embeddings-inference"),
    ("pg_vector", "PgVector", "http://127.0.0.1:5432", "PG_VECTOR_URL", True, "PostgreSQL pgvector", "github.com/pgvector/pgvector"),
    ("s3_vectors", "S3Vectors", "https://s3.amazonaws.com", "S3_VECTORS_API_KEY", False, "S3 Vectors", "aws.amazon.com"),
    ("milvus", "Milvus", "http://127.0.0.1:19530", "MILVUS_URL", True, "Milvus (向量库)", "milvus.io"),
    ("qdrant", "Qdrant", "http://127.0.0.1:6333", "QDRANT_URL", True, "Qdrant (向量库)", "qdrant.tech"),
    ("chatgpt", "Chatgpt", "https://chatgpt.com/backend-api/codex", "CHATGPT_API_KEY", False, "ChatGPT (订阅)", "openai.com"),
    ("nanogpt", "Nanogpt", "https://api.nanogpt.com/v1", "NANOGPT_API_KEY", False, "NanoGPT", "nanogpt.com"),
    ("local", "Local", "http://127.0.0.1:8080/v1", "LOCAL_LLM_BASE_URL", True, "Local LLM", "localhost"),
    ("cybertron", "Cybertron", "http://127.0.0.1:8080/v1", "CYBERTRON_BASE_URL", True, "Cybertron (Rust)", "github.com/dottorblaster/cybertron"),
    ("mlx", "Mlx", "http://127.0.0.1:8080/v1", "MLX_BASE_URL", True, "MLX (Apple Silicon)", "github.com/ml-explore/mlx"),
    ("openvino", "Openvino", "http://127.0.0.1:8080/v1", "OPENVINO_BASE_URL", True, "OpenVINO", "intel.com/openvino"),
    ("gaudi", "Gaudi", "http://127.0.0.1:8080/v1", "GAUDI_BASE_URL", True, "Intel Gaudi", "intel.com"),
    ("onnx", "Onnx", "http://127.0.0.1:8080/v1", "ONNX_BASE_URL", True, "ONNX Runtime", "onnxruntime.ai"),
    ("omlx", "Omlx", "http://127.0.0.1:8080/v1", "OMLX_BASE_URL", True, "OMLX / MLX LM", "github.com/ml-explore/mlx-lm"),
    ("doubaoaudio", "DoubaoAudio", "https://ark.cn-beijing.volces.com/api/v3", "ARK_API_KEY", False, "Doubao Audio", "volces.com"),
]

def gen_local_template(mod, prefix, base_url, env_var, display, doc_url):
    return f'''//! {display} provider — a thin OpenAI-compatible wrapper.
//!
//! See <{doc_url}> for API documentation. Exposes an OpenAI-compatible
//! Chat Completions API at `{base_url}`. The `{env_var}` environment
//! variable holds a *base URL* (not an API key); when unset, the default
//! endpoint is used. A placeholder API key is sent in the `Authorization`
//! header — the shared `OpenAIProvider` requires a non-empty key string.

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;

use crate::openai::{{OpenAIConfig, OpenAIModel, OpenAIProvider}};

const DEFAULT_BASE_URL: &str = "{base_url}";
const ENV_VAR: &str = "{env_var}";
const PROVIDER_NAME: &str = "{mod}";
const PLACEHOLDER_API_KEY: &str = "{mod}";

pub struct {prefix}Config(OpenAIConfig);

impl {prefix}Config {{
    pub fn new(api_key: impl Into<String>) -> Self {{
        Self(
            OpenAIConfig::new(api_key)
                .with_base_url(DEFAULT_BASE_URL)
                .with_provider(PROVIDER_NAME),
        )
    }}

    pub fn from_env() -> Result<Self, AiMuxError> {{
        let config = Self::new(PLACEHOLDER_API_KEY);
        match std::env::var(ENV_VAR) {{
            Ok(url) if !url.trim().is_empty() => Ok(config.with_base_url(url)),
            _ => Ok(config),
        }}
    }}

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {{
        self.0 = self.0.with_base_url(url);
        self
    }}
}}

pub struct {prefix}Provider(OpenAIProvider);

impl {prefix}Provider {{
    pub fn new(config: {prefix}Config) -> Self {{
        Self(OpenAIProvider::new(config.0))
    }}

    pub fn model(&self, model_id: &str) -> OpenAIModel {{
        self.0.model(model_id)
    }}
}}

impl Provider for {prefix}Provider {{
    fn name(&self) -> &str {{
        PROVIDER_NAME
    }}

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {{
        Ok(Box::new(self.model(model_id)))
    }}
}}
'''

def gen_cloud_template(mod, prefix, base_url, env_var, display, doc_url):
    return f'''//! {display} provider — a thin OpenAI-compatible wrapper.
//!
//! See <{doc_url}> for API documentation. Exposes an OpenAI-compatible
//! Chat Completions API at `{base_url}`. Provider-specific details are the
//! base URL and the `{env_var}` environment variable; everything else is
//! delegated to the shared `OpenAIProvider`.

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::load_api_key;

use crate::openai::{{OpenAIConfig, OpenAIModel, OpenAIProvider}};

const DEFAULT_BASE_URL: &str = "{base_url}";
const ENV_VAR: &str = "{env_var}";
const PROVIDER_NAME: &str = "{mod}";

pub struct {prefix}Config(OpenAIConfig);

impl {prefix}Config {{
    pub fn new(api_key: impl Into<String>) -> Self {{
        Self(OpenAIConfig::new(api_key).with_base_url(DEFAULT_BASE_URL))
    }}

    pub fn from_env() -> Result<Self, AiMuxError> {{
        let key = load_api_key(None, ENV_VAR, "{display}")?;
        Ok(Self::new(key))
    }}

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {{
        self.0 = self.0.with_base_url(url);
        self
    }}
}}

pub struct {prefix}Provider(OpenAIProvider);

impl {prefix}Provider {{
    pub fn new(config: {prefix}Config) -> Self {{
        Self(OpenAIProvider::new(config.0))
    }}

    pub fn model(&self, model_id: &str) -> OpenAIModel {{
        self.0.model(model_id)
    }}
}}

impl Provider for {prefix}Provider {{
    fn name(&self) -> &str {{
        PROVIDER_NAME
    }}

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {{
        Ok(Box::new(self.model(model_id)))
    }}
}}
'''

def main():
    seen = set()
    generated = []
    skipped = []

    for mod_name, prefix, base_url, env_var, is_local, display, doc_url in PROVIDERS:
        if mod_name in ALREADY_DONE or mod_name in seen:
            skipped.append(mod_name)
            continue
        seen.add(mod_name)

        out_path = SRC / f"{mod_name}.rs"
        if out_path.exists():
            skipped.append(mod_name)
            continue

        if is_local:
            content = gen_local_template(mod_name, prefix, base_url, env_var, display, doc_url)
        else:
            content = gen_cloud_template(mod_name, prefix, base_url, env_var, display, doc_url)

        out_path.write_text(content, encoding="utf-8")
        generated.append((mod_name, prefix))

    print(f"Generated {len(generated)} provider files:")
    for mod, prefix in sorted(generated):
        print(f"  {mod}.rs ({prefix}Config / {prefix}Provider)")
    if skipped:
        print(f"\nSkipped {len(skipped)} (already exist): {', '.join(sorted(skipped))}")

    # Print lib.rs snippet
    print("\n# Add to lib.rs:")
    for mod_name, _ in sorted(generated):
        print(f'pub mod {mod_name};')
    print()
    for mod_name, prefix in sorted(generated):
        print(f'pub use {mod_name}::{{{prefix}Config, {prefix}Provider}};')

if __name__ == "__main__":
    main()
