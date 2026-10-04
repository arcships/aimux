# aimux providers

> **GENERATED** by `scripts/gen_providers_doc.py` — do not edit by hand.
> Regenerate with: `python scripts/gen_providers_doc.py`
> CI verifies with `--check`; the totals below are the one source of
> truth for provider counts (#177).

## Totals

| category | count |
|----------|-------|
| Registry-backed OpenAI-compatible (`provider(name, ...)`) | 281 |
| Non-registry: Native protocol providers | 13 |
| Non-registry: Speech-only providers (TTS) | 4 |
| Non-registry: Transcription-only providers (STT) | 5 |
| Non-registry: Image-only providers | 4 |
| Non-registry: Video-only providers | 1 |
| Non-registry: Generic Responses API wrapper | 1 |
| Non-registry: Modality-specific providers (non-language, e.g. rerank-only) | 1 |
| Non-registry: AWS Polly speech (TTS) provider — SigV4 authenticated, speech modality only | 1 |
| Non-registry: Recraft image provider (OpenAI Images-compatible + Recraft extension fields) | 1 |
| Non-registry: Stability image provider (image modality only) | 1 |
| Non-registry: Video-only provider (runwayml) | 1 |
| Non-registry: Search-only providers (web search modality) | 11 |
| **Total providers** | **325** |

**281 registry-backed OpenAI-compatible providers** (create presets by name via `provider(name, ...)`) + **44 non-registry providers** (construct via the typed factories listed below).

## Registry-backed (OpenAI-compatible) — 281

| name | display | auth | env var | base_url |
|------|---------|------|---------|----------|
| `abacus` | Abacus | api_key | `ABACUS_API_KEY` | `https://routellm.abacus.ai/v1` |
| `abliteration_ai` | Abliteration AI | api_key | `ABLIT_KEY` | `https://api.abliteration.ai/v1` |
| `ai21` | AI21 Labs | api_key | `AI21_API_KEY` | `https://api.ai21.ai/v1` |
| `ai302` | 302.AI | api_key | `AI302_API_KEY` | `https://api.302.ai/v1` |
| `ai_router` | AI-ROUTER | api_key | `AI_ROUTER_API_KEY` | `https://api.ai-router.dev/v1` |
| `aiand` | AIand | api_key | `AIAND_API_KEY` | `https://api.aiand.com/v1` |
| `aibadgr` | AI Badgr | api_key | `AIBADGR_API_KEY` | `https://api.aibadgr.com/v1` |
| `aigc2d` | AIGC2D | api_key | `AIGC2D_API_KEY` | `https://api.aigc2d.com/v1` |
| `aihubmix` | AIHubMix | api_key | `AIHUBMIX_API_KEY` | `https://aihubmix.com/v1` |
| `ails` | AILS | api_key | `AILS_API_KEY` | `https://api.caipacity.com/v1` |
| `aiml` | AI/ML API | api_key | `AIML_API_KEY` | `https://api.aimlapi.com/v1` |
| `aki_io` | AKI.IO | api_key | `AKI_IO_API_KEY` | `https://aki.io/openai/v1` |
| `albert` | Albert | api_key | `ALBERT_API_KEY` | `https://api.albert.ai/v1` |
| `alibaba` | Alibaba Cloud (DashScope) | api_key | `ALIBABA_API_KEY` | `https://dashscope-intl.aliyuncs.com/compatible-mode/v1` |
| `alibaba_coding_plan` | Alibaba Coding Plan | api_key | `ALIBABA_CODING_PLAN_API_KEY` | `https://coding-intl.dashscope.aliyuncs.com/v1` |
| `alibaba_coding_plan_cn` | Alibaba Coding Plan (China) | api_key | `ALIBABA_CODING_PLAN_API_KEY` | `https://coding.dashscope.aliyuncs.com/v1` |
| `alibaba_token_plan` | Alibaba Token Plan | api_key | `ALIBABA_TOKEN_PLAN_API_KEY` | `https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1` |
| `alibaba_token_plan_cn` | Alibaba Token Plan (China) | api_key | `ALIBABA_TOKEN_PLAN_API_KEY` | `https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1` |
| `ambient` | Ambient | api_key | `AMBIENT_API_KEY` | `https://api.ambient.xyz/v1` |
| `anyapi` | AnyAPI | api_key | `ANYAPI_KEY` | `https://api.anyapi.ai/v1` |
| `anyscale` | Anyscale | api_key | `ANYSCALE_API_KEY` | `https://api.endpoints.anyscale.com/v1` |
| `apertis` | Apertis | api_key | `STIMA_API_KEY` | `https://api.stima.tech/v1` |
| `api2d` | API2D | api_key | `API2D_API_KEY` | `https://oa.api2d.net/v1` |
| `api2gpt` | API2GPT | api_key | `API2GPT_API_KEY` | `https://api.api2gpt.com/v1` |
| `apiserpent` | API Serpent | api_key | `APISERPENT_API_KEY` | `https://api.apiserpent.com/v1` |
| `atlascloud` | AtlasCloud | api_key | `ATLASCLOUD_API_KEY` | `https://api.atlascloud.com/v1` |
| `atomic_chat` | Atomic Chat | api_key | `ATOMIC_CHAT_API_KEY` | `http://127.0.0.1:1337/v1` |
| `auriko` | Auriko | api_key | `AURIKO_API_KEY` | `https://api.auriko.ai/v1` |
| `azure_ai` | Azure AI | api_key | `AZURE_AI_API_KEY` | `https://models.inference.ai.azure.com` |
| `baichuan` | Baichuan AI | api_key | `BAICHUAN_API_KEY` | `https://api.baichuan-ai.com/v1` |
| `baidu` | Baidu (文心/ERNIE) | api_key | `BAIDU_API_KEY` | `https://qianfan.baidubce.com/v2` |
| `baidu_v2` | BaiduV2 | api_key | `QIANFAN_API_KEY` | `https://qianfan.baidubce.com/v2` |
| `bailing` | Bailing | api_key | `BAILING_API_TOKEN` | `https://api.ant-ling.com/v1` |
| `baseten` | Baseten | api_key | `BASETEN_API_KEY` | `https://inference.baseten.co/v1` |
| `bedrock_mantle` | Bedrock Mantle | api_key | `BEDROCK_MANTLE_API_KEY` | `https://bedrock-mantle.{region}.api.aws/v1` (params: `region`) |
| `berget` | Berget.AI | api_key | `BERGET_API_KEY` | `https://api.berget.ai/v1` |
| `bigmodel` | BigModel (智谱) | api_key | `BIGMODEL_API_KEY` | `https://open.bigmodel.cn/api/paas/v4` |
| `blueclaw` | Blue Claw | api_key | `BLUECLAW_API_KEY` | `https://openai.blueclaw.network/v1` |
| `bytedance` | ByteDance | api_key | `ARK_API_KEY` | `https://ark.cn-beijing.volces.com/api/v3` |
| `byteplus` | BytePlus (Volcano) | api_key | `BYTEPLUS_API_KEY` | `https://ark.bytepluses.com/api/v3` |
| `bytez` | Bytez | api_key | `BYTEZ_API_KEY` | `https://api.bytez.com/v2` |
| `canopywave` | Canopywave | api_key | `CANOPYWAVE_API_KEY` | `https://api.canopywave.com/v1` |
| `cerebras` | Cerebras | api_key | `CEREBRAS_API_KEY` | `https://api.cerebras.ai/v1` |
| `chatgpt` | ChatGPT (订阅) | api_key | `CHATGPT_API_KEY` | `https://chatgpt.com/backend-api/codex` |
| `cherryin` | cherryin | api_key | `CHERRYIN_API_KEY` | `https://open.cherryin.net` |
| `chutes` | Chutes | api_key | `CHUTES_API_KEY` | `https://llm.chutes.ai/v1` |
| `clarifai` | Clarifai | api_key | `CLARIFAI_API_KEY` | `https://api.clarifai.com/v2/ext/openai/v1` |
| `claudinio` | Claudinio | api_key | `CLAUDINIO_API_KEY` | `https://api.claudin.io` |
| `cline_pass` | Cline | api_key | `CLINE_API_KEY` | `https://api.cline.bot/v1` |
| `closeai` | CloseAI | api_key | `CLOSEAI_API_KEY` | `https://api.closeai-proxy.xyz/v1` |
| `cloudferro_sherlock` | CloudFerro Sherlock | api_key | `CLOUDFERRO_SHERLOCK_API_KEY` | `https://api-sherlock.cloudferro.com/openai/v1` |
| `cloudflare` | Cloudflare | api_key | `CLOUDFLARE_API_KEY` | `https://api.cloudflare.com/client/v4/accounts/{account_id}/ai/v1` (params: `account_id`) |
| `cloudflare_workers_ai` | Cloudflare Workers AI | api_key | `CLOUDFLARE_API_KEY` | `https://api.cloudflare.com/client/v4/accounts/{account_id}/ai/v1` (params: `account_id`) |
| `codestral` | Codestral (Mistral) | api_key | `CODESTRAL_API_KEY` | `https://api.mistral.ai/v1` |
| `cometapi` | CometAPI | api_key | `COMETAPI_API_KEY` | `https://api.cometapi.com/v1` |
| `commandcode` | CommandCode | api_key | `COMMANDCODE_API_KEY` | `https://api.commandcode.com/v1` |
| `compactifai` | CompactifAI | api_key | `COMPACTIFAI_API_KEY` | `https://api.compactif.ai/v1` |
| `copilot` | GitHub Copilot | api_key | `COPILOT_API_KEY` | `https://api.githubcopilot.com` |
| `cortecs` | Cortecs | api_key | `CORTECS_API_KEY` | `https://api.cortecs.ai/v1/` |
| `coze` | Coze (扣子) | api_key | `COZE_API_KEY` | `https://api.coze.cn/v1` |
| `crof` | CrofAI | api_key | `CROF_API_KEY` | `https://crof.ai/v1` |
| `crossmodel` | CrossModel | api_key | `CROSSMODEL_API_KEY` | `https://api.crossmodel.ai/v1` |
| `crusoe` | Crusoe | api_key | `CRUSOE_API_KEY` | `https://api.inference.crusoecloud.com/v1` |
| `cybertron` | Cybertron | none | — | `http://127.0.0.1:8080/v1` (URL from `CYBERTRON_BASE_URL`) |
| `daoxe` | DaoXE | api_key | `DAOXE_API_KEY` | `https://daoxe.com/v1` |
| `darkbloom` | Darkbloom | api_key | `DARKBLOOM_API_KEY` | `https://api.darkbloom.dev/v1` |
| `databricks` | Databricks | api_key | `DATABRICKS_API_KEY` | `https://databricks.com/serving-endpoints` |
| `datarobot` | DataRobot | api_key | `DATAROBOT_API_TOKEN` | `https://app.datarobot.com/api/v2` |
| `deepbricks` | DeepBricks | api_key | `DEEPBRICKS_API_KEY` | `https://api.deepbricks.ai/v1` |
| `deepinfra` | DeepInfra | api_key | `DEEPINFRA_API_KEY` | `https://api.deepinfra.com/v1/openai` |
| `digitalocean` | DigitalOcean | api_key | `DIGITALOCEAN_ACCESS_TOKEN` | `https://inference.do-ai.run` |
| `dinference` | DInference | api_key | `DINFERENCE_API_KEY` | `https://api.dinference.com/v1` |
| `docker_model_runner` | Docker Model Runner | none | — | `http://model-runner.docker.internal/engines/llama.cpp/v1` (URL from `DOCKER_MODEL_RUNNER_BASE_URL`) |
| `doubao` | Doubao | api_key | `ARK_API_KEY` | `https://ark.cn-beijing.volces.com/api/v3` |
| `doubleword` | Doubleword | api_key | `DOUBLEWORD_API_KEY` | `https://api.doubleword.ai/v1` |
| `drun` | D.Run (China) | api_key | `DRUN_API_KEY` | `https://chat.d.run/v1` |
| `ebcloud` | EBCloud | api_key | `EBCLOUD_API_KEY` | `https://maas-api.ebcloud.com/v1` |
| `embercloud` | Embercloud | api_key | `EMBERCLOUD_API_KEY` | `https://api.embercloud.com/v1` |
| `empiriolabs` | EmpirioLabs AI | api_key | `EMPIRIOLABS_API_KEY` | `https://api.empiriolabs.ai/v1` |
| `evroc` | evroc | api_key | `EVROC_API_KEY` | `https://models.think.evroc.com/v1` |
| `fastcrw` | FastCRW | api_key | `FASTCRW_API_KEY` | `https://fastcrw.com/api/v1` |
| `fastgpt` | FastGPT | api_key | `FASTGPT_API_KEY` | `https://api.fastgpt.in/v1` |
| `fastrouter` | FastRouter | api_key | `FASTROUTER_API_KEY` | `https://api.fastrouter.ai/v1` |
| `featherless_ai` | Featherless AI | api_key | `FEATHERLESS_API_KEY` | `https://api.featherless.ai/v1` |
| `firepass` | Fireworks (Firepass) | api_key | `FIREWORKS_API_KEY` | `https://api.fireworks.ai/inference/v1` |
| `fireworks` | Fireworks | api_key | `FIREWORKS_API_KEY` | `https://api.fireworks.ai/inference/v1` |
| `freemodel` | FreeModel | api_key | `FREEMODEL_API_KEY` | `https://api.freemodel.dev/v1` |
| `friendliai` | FriendliAI | api_key | `FRIENDLIAI_API_KEY` | `https://inference.friendli.ai/v1` |
| `frogbot` | FrogBot | api_key | `FROGBOT_API_KEY` | `https://app.frogbot.ai/api/v1` |
| `galadriel` | Galadriel | api_key | `GALADRIEL_API_KEY` | `https://api.galadriel.com/v1` |
| `gaudi` | Intel Gaudi | none | — | `http://127.0.0.1:8080/v1` (URL from `GAUDI_BASE_URL`) |
| `gdc` | GDC | api_key | `GDC_API_KEY` | `https://api.gdc.ai/v1` |
| `gigachat` | GigaChat (Sberbank) | api_key | `GIGACHAT_API_KEY` | `https://gigachat.devices.sberbank.ru/api/v1` |
| `github` | GitHub Models | api_key | `GITHUB_TOKEN` | `https://models.inference.ai.azure.com` |
| `gmi` | GMI | api_key | `GMI_API_KEY` | `https://api.gmi-serving.com/v1` |
| `gmicloud` | GMI Cloud | api_key | `GMI_API_KEY` | `https://api.gmi-serving.com/v1` |
| `gonka24` | Gonka24 | api_key | `GONKA24_API_KEY` | `https://api.gonka24.com/v1` |
| `gradient_ai` | Gradient AI | api_key | `GRADIENT_API_KEY` | `https://inference.do-ai.run/v1` |
| `helicone` | Helicone | api_key | `HELICONE_API_KEY` | `https://api.helicone.ai/v1` |
| `heroku` | Heroku AI | api_key | `HEROKU_API_KEY` | `https://api.heroku.com/inference/v1` |
| `hetzner` | Hetzner | api_key | `HETZNER_VLLM_API_KEY` | `https://inference.hetzner.com/api/v1` |
| `hosted_vllm` | Hosted vLLM | api_key | `HOSTED_VLLM_API_KEY` | `https://hosted-vllm-api.com/v1` |
| `hpc_ai` | HPC-AI | api_key | `INFERENCE_API_KEY` | `https://api.hpc-ai.com/inference/v1` |
| `hyperbolic` | Hyperbolic | api_key | `HYPERBOLIC_API_KEY` | `https://api.hyperbolic.xyz/v1` |
| `iflowcn` | iFlow | api_key | `IFLOW_API_KEY` | `https://apis.iflow.cn/v1` |
| `inception` | Inception Labs | api_key | `INCEPTION_API_KEY` | `https://api.inceptionlabs.ai/v1` |
| `inceptron` | Inceptron | api_key | `INCEPTRON_API_KEY` | `https://api.inceptron.io/v1` |
| `inference_net` | Inference.net | api_key | `INFERENCE_NET_API_KEY` | `https://api.inference.net/v1` |
| `inferencehub` | InferenceHub | api_key | `INFERENCEHUB_API_KEY` | `https://app.inferencehub.tech/v1` |
| `inferx` | InferX | api_key | `INFERX_API_KEY` | `https://model.inferx.net/v1` |
| `infinity` | Infinity AI | api_key | `INFINITY_API_KEY` | `https://infinity.ai/api/v1` |
| `io_net` | IO.NET | api_key | `IOINTELLIGENCE_API_KEY` | `https://api.intelligence.io.solutions/api/v1` |
| `jiekou` | Jiekou.AI | api_key | `JIEKOU_API_KEY` | `https://api.highwayapi.ai/openai` |
| `jlama` | Jlama | none | — | `http://127.0.0.1:8080/v1` (URL from `JLAMA_BASE_URL`) |
| `kenari` | Kenari | api_key | `KENARI_API_KEY` | `https://kenari.id/v1` |
| `kilo` | Kilo | api_key | `KILO_API_KEY` | `https://api.kilo.ai/v1` |
| `kimi` | Kimi | api_key | `MOONSHOT_API_KEY` | `https://api.moonshot.ai/v1` |
| `kimi_for_coding` | Kimi For Coding | api_key | `KIMI_API_KEY` | `https://api.kimi.com/coding/v1` |
| `kiro` | Kiro | api_key | `KIRO_API_KEY` | `https://api.kiro.dev/v1` |
| `kluster_ai` | Kluster AI | api_key | `KLUSTER_API_KEY` | `https://api.kluster.ai/v1` |
| `krutrim` | Krutrim | api_key | `KRUTRIM_API_KEY` | `https://api.krutrim.ai/v1` |
| `kuae_cloud_coding_plan` | KUAE Cloud Coding Plan | api_key | `KUAE_API_KEY` | `https://coding-plan-endpoint.kuaecloud.net/v1` |
| `lambda_ai` | Lambda AI | api_key | `LAMBDA_API_KEY` | `https://api.lambda.ai/v1` |
| `lemonade` | Lemonade | api_key | `LEMONADE_API_KEY` | `http://localhost:13305/v1` |
| `lemonfox_ai` | Lemonfox AI | api_key | `LEMONFOX_API_KEY` | `https://api.lemonfox.ai/v1` |
| `libertai` | Libertai | api_key | `LIBERTAI_API_KEY` | `https://api.libertai.io/v1` |
| `lilac` | Lilac | api_key | `LILAC_API_KEY` | `https://api.getlilac.com/v1` |
| `lingyiwanwu` | Lingyiwanwu (零一万物) | api_key | `LINGYIWANWU_API_KEY` | `https://api.lingyiwanwu.com/v1` |
| `litellm_proxy` | LiteLLM Proxy | none | — | `http://127.0.0.1:4000/v1` (URL from `LITELLM_PROXY_BASE_URL`) |
| `llama` | Llama | api_key | `LLAMA_API_KEY` | `https://api.llama.com/compat/v1/` |
| `llamacpp` | llama.cpp | none | — | `http://127.0.0.1:8080/v1` (URL from `LLAMACPP_BASE_URL`) |
| `llamafile` | Llamafile | none | — | `http://127.0.0.1:8080/v1` (URL from `LLAMAFILE_BASE_URL`) |
| `llamagate` | Llamagate | api_key | `LLAMAGATE_API_KEY` | `https://api.llamagate.dev/v1` |
| `llmgateway` | LLM Gateway | api_key | `LLM_GATEWAY_API_KEY` | `https://api.llmgateway.io/v1` |
| `llmtr` | LLMTR | api_key | `LLMTR_API_KEY` | `https://llmtr.com/v1` |
| `lmstudio` | LM Studio | none | — | `http://127.0.0.1:1234/v1` (URL from `LMSTUDIO_BASE_URL`) |
| `local` | Local LLM | none | — | `http://127.0.0.1:8080/v1` (URL from `LOCAL_LLM_BASE_URL`) |
| `localai` | LocalAI | none | — | `http://127.0.0.1:8080/v1` (URL from `LOCALAI_BASE_URL`) |
| `longcat` | LongCat | api_key | `LONGCAT_API_KEY` | `https://api.longcat.chat/v1` |
| `lucidquery` | LucidQuery | api_key | `LUCIDQUERY_API_KEY` | `https://api.lucidquery.com/v1` |
| `lynkr` | Lynkr | api_key | `LYNKR_API_KEY` | `http://localhost:8081/v1` |
| `matterai` | Matter AI | api_key | `MATTERAI_API_KEY` | `https://api.matterai.com/v1` |
| `meganova` | Meganova | api_key | `MEGANOVA_API_KEY` | `https://api.meganova.ai/v1` |
| `merge_gateway` | Merge Gateway | api_key | `MERGE_GATEWAY_API_KEY` | `https://api-gateway.merge.dev/v1/openai` |
| `meta` | Meta | api_key | `MODEL_API_KEY` | `https://api.meta.ai/v1` |
| `meta_llama` | Meta Llama API | api_key | `LLAMA_API_KEY` | `https://api.llama.com/compat/v1` |
| `mimo` | Mimo | api_key | `MIMO_API_KEY` | `https://api.xiaomimimo.com/v1` |
| `minimax` | MiniMax | api_key | `MINIMAX_API_KEY` | `https://api.minimax.io/v1` |
| `minimax_cn` | MiniMax (minimaxi.com) | api_key | `MINIMAX_API_KEY` | `https://api.minimaxi.com/v1` |
| `minimax_cn_coding_plan` | MiniMax Token Plan (minimaxi.com) | api_key | `MINIMAX_API_KEY` | `https://api.minimaxi.com/v1` |
| `minimax_coding_plan` | MiniMax Token Plan (minimax.io) | api_key | `MINIMAX_API_KEY` | `https://api.minimax.io/anthropic/v1` |
| `mira` | Mira | api_key | `MIRA_API_KEY` | `https://api.mira.so/v1` |
| `mistralrs` | Mistral.rs | none | — | `http://127.0.0.1:8080/v1` (URL from `MISTRALRS_BASE_URL`) |
| `mixlayer` | Mixlayer | api_key | `MIXLAYER_API_KEY` | `https://models.mixlayer.ai/v1` |
| `mlx` | MLX (Apple Silicon) | none | — | `http://127.0.0.1:8080/v1` (URL from `MLX_BASE_URL`) |
| `moark` | Moark | api_key | `MOARK_API_KEY` | `https://api.moark.com/v1` |
| `modal` | Modal | api_key | `MODAL_API_KEY` | `https://modal.com/v1` |
| `model_oracle_ai` | Model Oracle AI | api_key | `MODEL_ORACLE_API_KEY` | `https://api.modeloracle.com/api/v1` |
| `modelscope` | ModelScope | api_key | `MODELSCOPE_API_KEY` | `https://api-inference.modelscope.cn/v1` |
| `moonshotai` | Moonshot AI | api_key | `MOONSHOT_API_KEY` | `https://api.moonshot.cn/v1` |
| `moonshotai_cn` | Moonshot AI (China) | api_key | `MOONSHOT_API_KEY` | `https://api.moonshot.cn/anthropic/v1` |
| `morph` | Morph LLM | api_key | `MORPH_API_KEY` | `https://api.morphllm.com/v1` |
| `nanogpt` | NanoGPT | api_key | `NANOGPT_API_KEY` | `https://api.nanogpt.com/v1` |
| `ncompass` | Ncompass | api_key | `NCOMPASS_API_KEY` | `https://api.ncompass.tech/v1` |
| `nearai` | NEAR AI Cloud | api_key | `NEARAI_API_KEY` | `https://cloud-api.near.ai/v1` |
| `nebius` | Nebius AI | api_key | `NEBIUS_API_KEY` | `https://api.studio.nebius.ai/v1` |
| `neon` | Neon | api_key | `NEON_AI_GATEWAY_TOKEN` | `https://{branch_host}/v1` (params: `branch_host`) |
| `neuralwatt` | Neuralwatt | api_key | `NEURALWATT_API_KEY` | `https://api.neuralwatt.com/v1` |
| `nextbit` | NextBit | api_key | `NEXTBIT_API_KEY` | `https://api.nextbit.ai/v1` |
| `nlp_cloud` | NLP Cloud | api_key | `NLPCLOUD_API_KEY` | `https://api.nlpcloud.io/v1` |
| `nous_research` | Nous Research | api_key | `NOUS_API_KEY` | `https://api.nousresearch.com/v1` |
| `novita` | Novita AI | api_key | `NOVITA_API_KEY` | `https://api.novita.ai/v1` |
| `nscale` | Nscale | api_key | `NSCALE_API_KEY` | `https://inference.api.nscale.com/v1` |
| `nvidia_nim` | NVIDIA NIM | api_key | `NVIDIA_API_KEY` | `https://integrate.api.nvidia.com/v1` |
| `oci` | OCI | api_key | `OCI_API_KEY` | `https://inference.generativeai.{region}.oci.oraclecloud.com/openai/v1` (params: `region`) |
| `ofox` | OfoxAI | api_key | `OFOX_API_KEY` | `https://api.ofox.ai/v1` |
| `ohmygpt` | OhMyGPT | api_key | `OHMYGPT_API_KEY` | `https://api.ohmygpt.com/v1` |
| `ollama` | Ollama | none | — | `http://127.0.0.1:11434/v1` (URL from `OLLAMA_BASE_URL`) |
| `ollama_cloud` | Ollama Cloud | api_key | `OLLAMA_CLOUD_API_KEY` | `https://api.ollama.com/v1` |
| `omlx` | OMLX / MLX LM | none | — | `http://127.0.0.1:8080/v1` (URL from `OMLX_BASE_URL`) |
| `onnx` | ONNX Runtime | none | — | `http://127.0.0.1:8080/v1` (URL from `ONNX_BASE_URL`) |
| `oobabooba` | Oobabooga Text Generation WebUI | none | — | `http://127.0.0.1:5000/v1` (URL from `OOBABOOBA_BASE_URL`) |
| `openaimax` | OpenAIMax | api_key | `OPENAIMAX_API_KEY` | `https://api.openaimax.com/v1` |
| `openaisb` | OpenAI-SB | api_key | `OPENAISB_API_KEY` | `https://api.openaisb.com/v1` |
| `opencode` | OpenCode Zen | api_key | `OPENCODE_API_KEY` | `https://api.opencode.zen/v1` |
| `opencode_go` | OpenCode Go | api_key | `OPENCODE_GO_API_KEY` | `https://api.opencode.dev/v1` |
| `opencode_zen` | OpenCode Zen | api_key | `OPENCODE_ZEN_API_KEY` | `https://api.opencode.zen/v1` |
| `openrouter` | OpenRouter | api_key | `OPENROUTER_API_KEY` | `https://openrouter.ai/api/v1` |
| `openvino` | OpenVINO | none | — | `http://127.0.0.1:8080/v1` (URL from `OPENVINO_BASE_URL`) |
| `orcarouter` | OrcaRouter | api_key | `ORCAROUTER_API_KEY` | `https://api.orcarouter.com/v1` |
| `ovhcloud` | OVHcloud AI | api_key | `OVHCLOUD_API_KEY` | `https://oai.endpoints.kepler.ai.cloud.ovh.net/v1` |
| `parasail` | Parasail | api_key | `PARASAIL_API_KEY` | `https://api.parasail.io/v1` |
| `perfxcloud` | PerfXCloud | api_key | `PERFXCLOUD_API_KEY` | `https://api.perfxcloud.com/v1` |
| `perplexity` | Perplexity | api_key | `PERPLEXITY_API_KEY` | `https://api.perplexity.ai` |
| `perplexity_agent` | Perplexity Agent | api_key | `PERPLEXITY_API_KEY` | `https://api.perplexity.ai/v1` |
| `petals` | Petals | api_key | `PETALS_API_KEY` | `https://api.petals.dev/v1` |
| `pinstripes` | Pinstripes | api_key | `PINSTRIPES_API_KEY` | `https://api.pinstripes.io/v1` |
| `pioneer` | Pioneer | api_key | `PIONEER_API_KEY` | `https://api.pioneer.ai/v1` |
| `poe` | Poe | api_key | `POE_API_KEY` | `https://api.poe.com/v1` |
| `poolside` | Poolside | api_key | `POOLSIDE_API_KEY` | `https://inference.poolside.ai/v1` |
| `portkey` | Portkey Gateway | api_key | `PORTKEY_API_KEY` | `https://api.portkey.ai/v1` |
| `ppinfra` | PPInfra（PPIO 派欧云） | api_key | `PPIO_API_KEY` | `https://api.ppio.com/openai` |
| `predibase` | Predibase | api_key | `PREDIBASE_API_KEY` | `https://serving.app.predibase.com/v1` |
| `privatemode_ai` | Privatemode AI | api_key | `PRIVATEMODE_API_KEY` | `http://localhost:8080/v1` |
| `publicai` | Publicai | api_key | `PUBLICAI_API_KEY` | `https://platform.publicai.co/v1` |
| `qihang_ai` | QiHang（启航 AI） | api_key | `QIHANG_API_KEY` | `https://api.qhaigc.net/v1` |
| `qihoo360` | 360 AI | api_key | `AI360_API_KEY` | `https://api.360.cn/v1` |
| `qiniu_ai` | Qiniu AI | api_key | `QINIU_API_KEY` | `https://api.qiniu.com/v1` |
| `regolo_ai` | Regolo AI | api_key | `REGOLO_API_KEY` | `https://api.regolo.ai/v1` |
| `reka_ai` | Reka AI | api_key | `REKA_API_KEY` | `https://api.reka.ai/v1` |
| `requesty` | Requesty | api_key | `REQUESTY_API_KEY` | `https://api.requesty.ai/v1` |
| `reve` | Reve | api_key | `REVE_API_KEY` | `https://api.reve.ai/v1` |
| `routing_run` | routing.run | api_key | `ROUTING_RUN_API_KEY` | `https://api.routing.run/v1` |
| `sakana` | Sakana AI | api_key | `SAKANA_API_KEY` | `https://api.sakana.ai/v1` |
| `sambanova` | SambaNova | api_key | `SAMBANOVA_API_KEY` | `https://api.sambanova.ai/v1` |
| `sarvam` | Sarvam AI | api_key | `SARVAM_API_KEY` | `https://api.sarvam.ai/v1` |
| `scaleway` | Scaleway AI | api_key | `SCALEWAY_API_KEY` | `https://api.scaleway.ai/v1` |
| `scx_ai` | SCX AI | api_key | `SCX_AI_API_KEY` | `https://api.scx.ai/v1` |
| `sglang` | SGLang | none | — | `http://127.0.0.1:30000/v1` (URL from `SGLANG_BASE_URL`) |
| `siliconflow` | SiliconFlow | api_key | `SILICONFLOW_API_KEY` | `https://api.siliconflow.cn/v1` |
| `snowflake` | Snowflake | api_key | `SNOWFLAKE_PAT` | `https://{account_identifier}.snowflakecomputing.com/api/v2/cortex/v1` (params: `account_identifier`) |
| `snowflake_cortex` | Snowflake Cortex | api_key | `SNOWFLAKE_CORTEX_PAT` | `https://{account_identifier}.snowflakecomputing.com/api/v2/cortex/v1` (params: `account_identifier`) |
| `stackit` | STACKIT | api_key | `STACKIT_API_KEY` | `https://api.openai-compat.model-serving.eu01.onstackit.cloud/v1` |
| `stepfun` | StepFun (阶跃星辰) | api_key | `STEPFUN_API_KEY` | `https://api.stepfun.com/v1` |
| `stepfun_ai_step_plan` | StepFun Step Plan (Global) | api_key | `STEPFUN_API_KEY` | `https://api.stepfun.ai/step_plan/v1` |
| `stepfun_step_plan` | StepFun Step Plan (China) | api_key | `STEPFUN_API_KEY` | `https://api.stepfun.com/step_plan/v1` |
| `subconscious` | Subconscious | api_key | `SUBCONSCIOUS_API_KEY` | `https://api.subconscious.dev/v1` |
| `submodel` | SubModel | api_key | `SUBMODEL_API_KEY` | `https://api.submodel.com/v1` |
| `synthetic` | Synthetic | api_key | `SYNTHETIC_API_KEY` | `https://api.synthetic.new/openai/v1` |
| `tencent` | Tencent (混元/Hunyuan) | api_key | `TENCENT_API_KEY` | `https://api.hunyuan.cloud.tencent.com/v1` |
| `tencent_coding_plan` | Tencent Coding Plan (China) | api_key | `TENCENT_CODING_PLAN_API_KEY` | `https://api.lkeap.cloud.tencent.com/coding/v3` |
| `tencent_token_plan` | Tencent Token Plan | api_key | `TENCENT_TOKEN_PLAN_API_KEY` | `https://api.lkeap.cloud.tencent.com/plan/v3` |
| `tencent_token_plan_enterprise_auto` | 腾讯云 Token Plan / Token Plan 企业版轻享套餐 | api_key | `TENCENT_TOKEN_PLAN_ENTERPRISE_API_KEY` | `https://tokenhub.tencentmaas.com/plan/v3` |
| `tencent_token_plan_enterprise_pro` | 腾讯云 Token Plan / Token Plan 企业版专业套餐 | api_key | `TENCENT_TOKEN_PLAN_ENTERPRISE_API_KEY` | `https://tokenhub.tencentmaas.com/plan/v3` |
| `tencent_token_plan_general_personal` | 腾讯云 Token Plan / 通用 Token Plan（个人版） | api_key | `TENCENT_TOKEN_PLAN_API_KEY` | `https://api.lkeap.cloud.tencent.com/plan/v3` |
| `tencent_token_plan_hy_personal` | 腾讯云 Token Plan / Hy Token Plan（个人版） | api_key | `TENCENT_TOKEN_PLAN_API_KEY` | `https://api.lkeap.cloud.tencent.com/plan/v3` |
| `tencent_tokenhub` | Tencent TokenHub | api_key | `TENCENT_TOKENHUB_API_KEY` | `https://tokenhub.tencentmaas.com/v1` |
| `tensormesh` | Tensormesh | api_key | `YOUR_API_KEY` | `https://serverless.tensormesh.ai` |
| `the_grid_ai` | The Grid AI | api_key | `THEGRIDAI_API_KEY` | `https://api.thegrid.ai/v1` |
| `thinkingmachines` | Thinking Machines | api_key | `TINKER_API_KEY` | `https://tinker.thinkingmachines.dev/services/tinker-prod/oai/api/v1` |
| `tinfoil` | Tinfoil | api_key | `TINFOIL_API_KEY` | `https://inference.tinfoil.sh/v1` |
| `togetherai` | Together AI | api_key | `TOGETHER_API_KEY` | `https://api.together.xyz/v1` |
| `tokenflux` | Tokenflux | api_key | `TOKENFLUX_API_KEY` | `https://tokenflux.ai/v1` |
| `tokenpony` | TokenPony | api_key | `TOKENPONY_API_KEY` | `https://api.tokenpony.com/v1` |
| `trustedrouter` | TrustedRouter | api_key | `TRUSTEDROUTER_API_KEY` | `https://api.trustedrouter.com/v1` |
| `tundra` | Tundra | api_key | `TUNDRA_API_KEY` | `https://api.tundra.ai/v1` |
| `umans_ai` | Umans AI | api_key | `UMANS_AI_API_KEY` | `https://api.code.umans.ai/v1` |
| `unorouter` | UnoRouter | api_key | `UNOROUTER_API_KEY` | `https://unorouter.com/en` |
| `upstage` | Upstage | api_key | `UPSTAGE_API_KEY` | `https://api.upstage.ai/v1` |
| `v0` | v0 (Vercel) | api_key | `V0_API_KEY` | `https://api.v0.dev/v1` |
| `venice` | Venice | api_key | `VENICE_API_KEY` | `https://api.venice.ai/api/v1` |
| `vercel` | Vercel | api_key | `VERCEL_API_KEY` | `https://api.v0.dev/v1` |
| `vertex_ai_ai21_models` | AI21 models on Vertex AI | api_key | `GOOGLE_VERTEX_ACCESS_TOKEN` | `https://{host}/v1/projects/{project}/locations/{location}/endpoints/openapi` (params: `project`, `location`) |
| `vertex_ai_anthropic_models` | Anthropic Claude models on Vertex AI | api_key | `GOOGLE_VERTEX_ACCESS_TOKEN` | `https://{host}/v1/projects/{project}/locations/{location}/endpoints/openapi` (params: `project`, `location`) |
| `vertex_ai_deepseek_models` | DeepSeek models on Vertex AI | api_key | `GOOGLE_VERTEX_ACCESS_TOKEN` | `https://{host}/v1/projects/{project}/locations/{location}/endpoints/openapi` (params: `project`, `location`) |
| `vertex_ai_llama_models` | Meta Llama models on Vertex AI | api_key | `GOOGLE_VERTEX_ACCESS_TOKEN` | `https://{host}/v1/projects/{project}/locations/{location}/endpoints/openapi` (params: `project`, `location`) |
| `vertex_ai_minimax_models` | MiniMax models on Vertex AI | api_key | `GOOGLE_VERTEX_ACCESS_TOKEN` | `https://{host}/v1/projects/{project}/locations/{location}/endpoints/openapi` (params: `project`, `location`) |
| `vertex_ai_mistral_models` | Mistral models on Vertex AI | api_key | `GOOGLE_VERTEX_ACCESS_TOKEN` | `https://{host}/v1/projects/{project}/locations/{location}/endpoints/openapi` (params: `project`, `location`) |
| `vertex_ai_moonshot_models` | Moonshot AI models on Vertex AI | api_key | `GOOGLE_VERTEX_ACCESS_TOKEN` | `https://{host}/v1/projects/{project}/locations/{location}/endpoints/openapi` (params: `project`, `location`) |
| `vertex_ai_openai_models` | OpenAI models on Vertex AI | api_key | `GOOGLE_VERTEX_ACCESS_TOKEN` | `https://{host}/v1/projects/{project}/locations/{location}/endpoints/openapi` (params: `project`, `location`) |
| `vertex_ai_qwen_models` | Qwen models on Vertex AI | api_key | `GOOGLE_VERTEX_ACCESS_TOKEN` | `https://{host}/v1/projects/{project}/locations/{location}/endpoints/openapi` (params: `project`, `location`) |
| `vertex_ai_zai_models` | Z.AI models on Vertex AI | api_key | `GOOGLE_VERTEX_ACCESS_TOKEN` | `https://{host}/v1/projects/{project}/locations/{location}/endpoints/openapi` (params: `project`, `location`) |
| `vivgrid` | Vivgrid | api_key | `VIVGRID_API_KEY` | `https://api.vivgrid.com/v1` |
| `vllm` | vLLM | none | — | `http://127.0.0.1:8000/v1` (URL from `VLLM_BASE_URL`) |
| `volc_engine` | VolcEngine | api_key | `ARK_API_KEY` | `https://ark.cn-beijing.volces.com/api/v3` |
| `vultr` | Vultr | api_key | `VULTR_API_KEY` | `https://api.vultrinference.com/v1` |
| `wafer` | Wafer | api_key | `WAFER_API_KEY` | `https://api.wafer.ai/v1` |
| `wandb` | Weights & Biases | api_key | `WANDB_API_KEY` | `https://api.inference.wandb.ai/v1` |
| `xiaomi_token_plan_ams` | Xiaomi Token Plan (Europe) | api_key | `MIMO_API_KEY` | `https://token-plan-ams.xiaomimimo.com/v1` |
| `xiaomi_token_plan_cn` | Xiaomi Token Plan (China) | api_key | `MIMO_API_KEY` | `https://token-plan-cn.xiaomimimo.com/v1` |
| `xiaomi_token_plan_sgp` | Xiaomi Token Plan (Singapore) | api_key | `MIMO_API_KEY` | `https://token-plan-sgp.xiaomimimo.com/v1` |
| `xiaomimimo` | Xiaomi MiMo | api_key | `XIAOMI_API_KEY` | `https://mimo.xiaomi.com/v1` |
| `xinference` | Xinference | none | — | `http://127.0.0.1:9997/v1` (URL from `XINFERENCE_BASE_URL`) |
| `xpersona` | Xpersona | api_key | `XPERSONA_API_KEY` | `https://www.xpersona.co/v1` |
| `xunfei` | Xunfei | api_key | `XUNFEI_API_PASSWORD` | `https://spark-api-open.xf-yun.com/v1` |
| `zai` | Zai | api_key | `ZAI_API_KEY` | `https://api.z.ai/api/paas/v4` |
| `zai_coding_plan` | Z.AI Coding Plan | api_key | `ZHIPU_API_KEY` | `https://api.z.ai/api/anthropic` |
| `zeldoc` | Zeldoc | api_key | `ZELDOC_API_KEY` | `https://api.zeldoc.ai/v1` |
| `zenmux` | ZenMux | api_key | `ZENMUX_API_KEY` | `https://zenmux.ai/api/v1` |
| `zhipu_v4` | ZhipuV4 | api_key | `ZHIPU_API_KEY` | `https://open.bigmodel.cn/api/paas/v4` |
| `zhipuai_coding_plan` | Zhipu AI Coding Plan | api_key | `ZHIPU_API_KEY` | `https://open.bigmodel.cn/api/coding/paas/v4` |

## Typed factories (non-registry)

These providers are **not** name-addressable: `provider("anthropic", ...)` fails with `NoSuchProvider`. Use the typed entry points below (Rust type names; per-binding constructors: see [reference.md](reference.md)).

### Native protocol providers — 13

| module | typed entry points |
|--------|--------------------|
| `anthropic` | `AnthropicProvider` / `AnthropicProviderSettings` / `create_anthropic` |
| `anthropic_aws` | `AnthropicAwsProvider` / `AnthropicAwsProviderSettings` / `create_anthropic_aws` |
| `azure` | `AzureOpenAIProvider` / `AzureOpenAIProviderSettings` / `create_azure` |
| `bedrock` | `AmazonBedrockProvider` / `AmazonBedrockProviderSettings` / `create_amazon_bedrock` |
| `cohere` | `CohereProvider` / `CohereProviderSettings` / `create_cohere` |
| `google` | `GoogleProvider` / `GoogleProviderSettings` / `create_google` |
| `mistral` | `MistralProvider` / `MistralProviderSettings` / `create_mistral` |
| `openai` | `OpenAIProvider` / `OpenAIProviderSettings` / `create_openai` |
| `vertex` | `VertexProvider` / `VertexProviderSettings` / `create_google_vertex` |
| `voyage` | `VoyageProvider` / `VoyageProviderSettings` / `create_voyage` |
| `codex` | `CodexProvider` / `CodexProviderSettings` / `create_codex` |
| `xai` | `XAIProvider` / `XAIProviderSettings` / `create_xai` |
| `huggingface` | `HuggingFaceProvider` / `HuggingFaceProviderSettings` / `create_huggingface` |

### Speech-only providers (TTS) — 4

| module | typed entry points |
|--------|--------------------|
| `cartesia` | `CartesiaProvider` / `CartesiaProviderSettings` / `create_cartesia` |
| `elevenlabs` | `ElevenLabsProvider` / `ElevenLabsProviderSettings` / `create_elevenlabs` |
| `hume` | `HumeProvider` / `HumeProviderSettings` / `create_hume` |
| `lmnt` | `LMNTProvider` / `LMNTProviderSettings` / `create_lmnt` |

### Transcription-only providers (STT) — 5

| module | typed entry points |
|--------|--------------------|
| `assemblyai` | `AssemblyAIProvider` / `AssemblyAIProviderSettings` / `create_assemblyai` |
| `deepgram` | `DeepgramProvider` / `DeepgramProviderSettings` / `create_deepgram` |
| `fal` | `FalProvider` / `FalProviderSettings` / `create_fal` |
| `gladia` | `GladiaProvider` / `GladiaProviderSettings` / `create_gladia` |
| `revai` | `RevaiProvider` / `RevaiProviderSettings` / `create_revai` |

### Image-only providers — 4

| module | typed entry points |
|--------|--------------------|
| `black_forest_labs` | `BlackForestLabsProvider` / `BlackForestLabsProviderSettings` / `create_black_forest_labs` |
| `luma` | `LumaProvider` / `LumaProviderSettings` / `create_luma` |
| `prodia` | `ProdiaProvider` / `ProdiaProviderSettings` / `create_prodia` |
| `replicate` | `ReplicateProvider` / `ReplicateProviderSettings` / `create_replicate` |

### Video-only providers — 1

| module | typed entry points |
|--------|--------------------|
| `klingai` | `KlingAIProvider` / `KlingAIProviderSettings` / `create_klingai` |

### Generic Responses API wrapper — 1

| module | typed entry points |
|--------|--------------------|
| `open_responses` | `OpenResponsesProvider` / `OpenResponsesProviderSettings` / `create_open_responses` |

### Modality-specific providers (non-language, e.g. rerank-only) — 1

| module | typed entry points |
|--------|--------------------|
| `jina_ai` | `JinaAiProvider` / `JinaAiProviderSettings` / `create_jina_ai` |

### AWS Polly speech (TTS) provider — SigV4 authenticated, speech modality only — 1

| module | typed entry points |
|--------|--------------------|
| `aws_polly` | `AwsPollyProvider` / `AwsPollyProviderSettings` / `create_aws_polly` |

### Recraft image provider (OpenAI Images-compatible + Recraft extension fields) — 1

| module | typed entry points |
|--------|--------------------|
| `recraft` | `RecraftProvider` / `RecraftProviderSettings` / `create_recraft` |

### Stability image provider (image modality only) — 1

| module | typed entry points |
|--------|--------------------|
| `stability` | `StabilityProvider` / `StabilityProviderSettings` / `create_stability` |

### Video-only provider (runwayml) — 1

| module | typed entry points |
|--------|--------------------|
| `runwayml` | `RunwaymlProvider` / `RunwaymlProviderSettings` / `create_runwayml` |

### Search-only providers (web search modality) — 11

| module | typed entry points |
|--------|--------------------|
| `dataforseo` | `DataforseoProvider` / `DataforseoProviderSettings` / `create_dataforseo` |
| `exa_ai` | `ExaAiProvider` / `ExaAiProviderSettings` / `create_exa_ai` |
| `firecrawl` | `FirecrawlProvider` / `FirecrawlProviderSettings` / `create_firecrawl` |
| `google_pse` | `GooglePseProvider` / `GooglePseProviderSettings` / `create_google_pse` |
| `linkup` | `LinkupProvider` / `LinkupProviderSettings` / `create_linkup` |
| `parallel_ai` | `ParallelAiProvider` / `ParallelAiProviderSettings` / `create_parallel_ai` |
| `searxng` | `SearxngProvider` / `SearxngProviderSettings` / `create_searxng` |
| `serper` | `SerperProvider` / `SerperProviderSettings` / `create_serper` |
| `tavily` | `TavilyProvider` / `TavilyProviderSettings` / `create_tavily` |
| `tinyfish` | `TinyfishProvider` / `TinyfishProviderSettings` / `create_tinyfish` |
| `you_com` | `YouComProvider` / `YouComProviderSettings` / `create_you_com` |

