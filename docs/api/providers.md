# aimux providers

> **GENERATED** by `scripts/gen_providers_doc.py` — do not edit by hand.
> Regenerate with: `python scripts/gen_providers_doc.py`

**251 registry-backed OpenAI-compatible providers** (construct via `provider(name, ...)` / `ProviderName`) + **78 non-registry providers** (construct via the typed factories listed below).

## Registry-backed (OpenAI-compatible) — 251

| name | display | env var | base_url |
|------|---------|---------|----------|
| `abacus` | Abacus | `ABACUS_API_KEY` | `https://routellm.abacus.ai/v1` |
| `abliteration_ai` | Abliteration AI | `ABLIT_KEY` | `https://api.abliteration.ai/v1` |
| `ai21` | AI21 Labs | `AI21_API_KEY` | `https://api.ai21.ai/v1` |
| `ai302` | 302.AI | `AI302_API_KEY` | `https://api.302.ai/v1` |
| `ai_router` | AI-ROUTER | `AI_ROUTER_API_KEY` | `https://api.ai-router.dev/v1` |
| `aiand` | AIand | `AIAND_API_KEY` | `https://api.aiand.com/v1` |
| `aibadgr` | AI Badgr | `AIBADGR_API_KEY` | `https://api.aibadgr.com/v1` |
| `aigc2d` | AIGC2D | `AIGC2D_API_KEY` | `https://api.aigc2d.com/v1` |
| `aihubmix` | AIHubMix | `AIHUBMIX_API_KEY` | `https://aihubmix.com/v1` |
| `ails` | AILS | `AILS_API_KEY` | `https://api.caipacity.com/v1` |
| `aiml` | AI/ML API | `AIML_API_KEY` | `https://api.aimlapi.com/v1` |
| `aki_io` | AKI.IO | `AKI_IO_API_KEY` | `https://aki.io/openai/v1` |
| `albert` | Albert | `ALBERT_API_KEY` | `https://api.albert.ai/v1` |
| `alibaba` | Alibaba Cloud (DashScope) | `ALIBABA_API_KEY` | `https://dashscope-intl.aliyuncs.com/compatible-mode/v1` |
| `alibaba_coding_plan` | Alibaba Coding Plan | `ALIBABA_CODING_PLAN_API_KEY` | `https://coding-intl.dashscope.aliyuncs.com/v1` |
| `alibaba_coding_plan_cn` | Alibaba Coding Plan (China) | `ALIBABA_CODING_PLAN_API_KEY` | `https://coding.dashscope.aliyuncs.com/v1` |
| `alibaba_token_plan` | Alibaba Token Plan | `ALIBABA_TOKEN_PLAN_API_KEY` | `https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1` |
| `alibaba_token_plan_cn` | Alibaba Token Plan (China) | `ALIBABA_TOKEN_PLAN_API_KEY` | `https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1` |
| `ambient` | Ambient | `AMBIENT_API_KEY` | `https://api.ambient.xyz/v1` |
| `anyapi` | AnyAPI | `ANYAPI_KEY` | `https://api.anyapi.ai/v1` |
| `anyscale` | Anyscale | `ANYSCALE_API_KEY` | `https://api.endpoints.anyscale.com/v1` |
| `apertis` | Apertis | `STIMA_API_KEY` | `https://api.stima.tech/v1` |
| `api2d` | API2D | `API2D_API_KEY` | `https://oa.api2d.net/v1` |
| `api2gpt` | API2GPT | `API2GPT_API_KEY` | `https://api.api2gpt.com/v1` |
| `apiserpent` | API Serpent | `APISERPENT_API_KEY` | `https://api.apiserpent.com/v1` |
| `atlascloud` | AtlasCloud | `ATLASCLOUD_API_KEY` | `https://api.atlascloud.com/v1` |
| `atomic_chat` | Atomic Chat | `ATOMIC_CHAT_API_KEY` | `http://127.0.0.1:1337/v1` |
| `auriko` | Auriko | `AURIKO_API_KEY` | `https://api.auriko.ai/v1` |
| `azure_ai` | Azure AI | `AZURE_AI_API_KEY` | `https://models.inference.ai.azure.com` |
| `baichuan` | Baichuan AI | `BAICHUAN_API_KEY` | `https://api.baichuan-ai.com/v1` |
| `baidu` | Baidu (文心/ERNIE) | `BAIDU_API_KEY` | `https://qianfan.baidubce.com/v2` |
| `baidu_v2` | BaiduV2 | `QIANFAN_API_KEY` | `https://qianfan.baidubce.com/v2` |
| `bailing` | Bailing | `BAILING_API_TOKEN` | `https://api.ant-ling.com/v1` |
| `baseten` | Baseten | `BASETEN_API_KEY` | `https://inference.baseten.co/v1` |
| `berget` | Berget.AI | `BERGET_API_KEY` | `https://api.berget.ai/v1` |
| `bigmodel` | BigModel (智谱) | `BIGMODEL_API_KEY` | `https://open.bigmodel.cn/api/paas/v4` |
| `blueclaw` | Blue Claw | `BLUECLAW_API_KEY` | `https://openai.blueclaw.network/v1` |
| `bytedance` | ByteDance | `ARK_API_KEY` | `https://ark.cn-beijing.volces.com/api/v3` |
| `byteplus` | BytePlus (Volcano) | `BYTEPLUS_API_KEY` | `https://ark.bytepluses.com/api/v3` |
| `bytez` | Bytez | `BYTEZ_API_KEY` | `https://api.bytez.com/v2` |
| `canopywave` | Canopywave | `CANOPYWAVE_API_KEY` | `https://api.canopywave.com/v1` |
| `cerebras` | Cerebras | `CEREBRAS_API_KEY` | `https://api.cerebras.ai/v1` |
| `chatgpt` | ChatGPT (订阅) | `CHATGPT_API_KEY` | `https://chatgpt.com/backend-api/codex` |
| `cherryin` | cherryin | `CHERRYIN_API_KEY` | `https://open.cherryin.net` |
| `chutes` | Chutes | `CHUTES_API_KEY` | `https://llm.chutes.ai/v1` |
| `clarifai` | Clarifai | `CLARIFAI_API_KEY` | `https://api.clarifai.com/v2/ext/openai/v1` |
| `claudinio` | Claudinio | `CLAUDINIO_API_KEY` | `https://api.claudin.io` |
| `cline_pass` | Cline | `CLINE_API_KEY` | `https://api.cline.bot/v1` |
| `closeai` | CloseAI | `CLOSEAI_API_KEY` | `https://api.closeai-proxy.xyz/v1` |
| `cloudferro_sherlock` | CloudFerro Sherlock | `CLOUDFERRO_SHERLOCK_API_KEY` | `https://api-sherlock.cloudferro.com/openai/v1` |
| `cloudflare` | Cloudflare | `CLOUDFLARE_API_KEY` | `https://api.cloudflare.com/client/v4/accounts/{CLOUDFLARE_ACCOUNT_ID}/ai/v1` |
| `cloudflare_workers_ai` | Cloudflare Workers AI | `CLOUDFLARE_API_KEY` | `https://api.cloudflare.com/client/v4/accounts/{CLOUDFLARE_ACCOUNT_ID}/ai/v1` |
| `codestral` | Codestral (Mistral) | `CODESTRAL_API_KEY` | `https://api.mistral.ai/v1` |
| `cometapi` | CometAPI | `COMETAPI_API_KEY` | `https://api.cometapi.com/v1` |
| `commandcode` | CommandCode | `COMMANDCODE_API_KEY` | `https://api.commandcode.com/v1` |
| `compactifai` | CompactifAI | `COMPACTIFAI_API_KEY` | `https://api.compactif.ai/v1` |
| `copilot` | GitHub Copilot | `COPILOT_API_KEY` | `https://api.githubcopilot.com` |
| `cortecs` | Cortecs | `CORTECS_API_KEY` | `https://api.cortecs.ai/v1/` |
| `coze` | Coze (扣子) | `COZE_API_KEY` | `https://api.coze.cn/v1` |
| `crof` | CrofAI | `CROF_API_KEY` | `https://crof.ai/v1` |
| `crossmodel` | CrossModel | `CROSSMODEL_API_KEY` | `https://api.crossmodel.ai/v1` |
| `crusoe` | Crusoe | `CRUSOE_API_KEY` | `https://api.inference.crusoecloud.com/v1` |
| `daoxe` | DaoXE | `DAOXE_API_KEY` | `https://daoxe.com/v1` |
| `darkbloom` | Darkbloom | `DARKBLOOM_API_KEY` | `https://api.darkbloom.dev/v1` |
| `databricks` | Databricks | `DATABRICKS_API_KEY` | `https://databricks.com/serving-endpoints` |
| `datarobot` | DataRobot | `DATAROBOT_API_TOKEN` | `https://app.datarobot.com/api/v2` |
| `deepbricks` | DeepBricks | `DEEPBRICKS_API_KEY` | `https://api.deepbricks.ai/v1` |
| `deepinfra` | DeepInfra | `DEEPINFRA_API_KEY` | `https://api.deepinfra.com/v1/openai` |
| `deepseek` | DeepSeek | `DEEPSEEK_API_KEY` | `https://api.deepseek.com/v1` |
| `digitalocean` | DigitalOcean | `DIGITALOCEAN_ACCESS_TOKEN` | `https://inference.do-ai.run` |
| `dinference` | DInference | `DINFERENCE_API_KEY` | `https://api.dinference.com/v1` |
| `doubao` | Doubao | `ARK_API_KEY` | `https://ark.cn-beijing.volces.com/api/v3` |
| `doubleword` | Doubleword | `DOUBLEWORD_API_KEY` | `https://api.doubleword.ai/v1` |
| `drun` | D.Run (China) | `DRUN_API_KEY` | `https://chat.d.run/v1` |
| `ebcloud` | EBCloud | `EBCLOUD_API_KEY` | `https://maas-api.ebcloud.com/v1` |
| `embercloud` | Embercloud | `EMBERCLOUD_API_KEY` | `https://api.embercloud.com/v1` |
| `empiriolabs` | EmpirioLabs AI | `EMPIRIOLABS_API_KEY` | `https://api.empiriolabs.ai/v1` |
| `evroc` | evroc | `EVROC_API_KEY` | `https://models.think.evroc.com/v1` |
| `fastcrw` | FastCRW | `FASTCRW_API_KEY` | `https://fastcrw.com/api/v1` |
| `fastgpt` | FastGPT | `FASTGPT_API_KEY` | `https://api.fastgpt.in/v1` |
| `fastrouter` | FastRouter | `FASTROUTER_API_KEY` | `https://api.fastrouter.ai/v1` |
| `featherless_ai` | Featherless AI | `FEATHERLESS_API_KEY` | `https://api.featherless.ai/v1` |
| `firepass` | Fireworks (Firepass) | `FIREWORKS_API_KEY` | `https://api.fireworks.ai/inference/v1` |
| `fireworks` | Fireworks | `FIREWORKS_API_KEY` | `https://api.fireworks.ai/inference/v1` |
| `freemodel` | FreeModel | `FREEMODEL_API_KEY` | `https://api.freemodel.dev/v1` |
| `friendliai` | FriendliAI | `FRIENDLIAI_API_KEY` | `https://inference.friendli.ai/v1` |
| `frogbot` | FrogBot | `FROGBOT_API_KEY` | `https://app.frogbot.ai/api/v1` |
| `galadriel` | Galadriel | `GALADRIEL_API_KEY` | `https://api.galadriel.com/v1` |
| `gdc` | GDC | `GDC_API_KEY` | `https://api.gdc.ai/v1` |
| `gigachat` | GigaChat (Sberbank) | `GIGACHAT_API_KEY` | `https://gigachat.devices.sberbank.ru/api/v1` |
| `github` | GitHub Models | `GITHUB_TOKEN` | `https://models.inference.ai.azure.com` |
| `gmi` | GMI | `GMI_API_KEY` | `https://api.gmi-serving.com/v1` |
| `gmicloud` | GMI Cloud | `GMI_API_KEY` | `https://api.gmi-serving.com/v1` |
| `gonka24` | Gonka24 | `GONKA24_API_KEY` | `https://api.gonka24.com/v1` |
| `gradient_ai` | Gradient AI | `GRADIENT_API_KEY` | `https://inference.do-ai.run/v1` |
| `groq` | Groq | `GROQ_API_KEY` | `https://api.groq.com/openai/v1` |
| `helicone` | Helicone | `HELICONE_API_KEY` | `https://api.helicone.ai/v1` |
| `heroku` | Heroku AI | `HEROKU_API_KEY` | `https://api.heroku.com/inference/v1` |
| `hetzner` | Hetzner | `HETZNER_VLLM_API_KEY` | `https://inference.hetzner.com/api/v1` |
| `hosted_vllm` | Hosted vLLM | `HOSTED_VLLM_API_KEY` | `https://hosted-vllm-api.com/v1` |
| `hpc_ai` | HPC-AI | `INFERENCE_API_KEY` | `https://api.hpc-ai.com/inference/v1` |
| `hyperbolic` | Hyperbolic | `HYPERBOLIC_API_KEY` | `https://api.hyperbolic.xyz/v1` |
| `iflowcn` | iFlow | `IFLOW_API_KEY` | `https://apis.iflow.cn/v1` |
| `inception` | Inception Labs | `INCEPTION_API_KEY` | `https://api.inceptionlabs.ai/v1` |
| `inceptron` | Inceptron | `INCEPTRON_API_KEY` | `https://api.inceptron.io/v1` |
| `inference_net` | Inference.net | `INFERENCE_NET_API_KEY` | `https://api.inference.net/v1` |
| `inferencehub` | InferenceHub | `INFERENCEHUB_API_KEY` | `https://app.inferencehub.tech/v1` |
| `inferx` | InferX | `INFERX_API_KEY` | `https://model.inferx.net/v1` |
| `infinity` | Infinity AI | `INFINITY_API_KEY` | `https://infinity.ai/api/v1` |
| `io_net` | IO.NET | `IOINTELLIGENCE_API_KEY` | `https://api.intelligence.io.solutions/api/v1` |
| `jiekou` | Jiekou.AI | `JIEKOU_API_KEY` | `https://api.highwayapi.ai/openai` |
| `kenari` | Kenari | `KENARI_API_KEY` | `https://kenari.id/v1` |
| `kilo` | Kilo | `KILO_API_KEY` | `https://api.kilo.ai/v1` |
| `kimi` | Kimi | `MOONSHOT_API_KEY` | `https://api.moonshot.ai/v1` |
| `kimi_for_coding` | Kimi For Coding | `KIMI_API_KEY` | `https://api.kimi.com/coding/v1` |
| `kiro` | Kiro | `KIRO_API_KEY` | `https://api.kiro.dev/v1` |
| `kluster_ai` | Kluster AI | `KLUSTER_API_KEY` | `https://api.kluster.ai/v1` |
| `krutrim` | Krutrim | `KRUTRIM_API_KEY` | `https://api.krutrim.ai/v1` |
| `kuae_cloud_coding_plan` | KUAE Cloud Coding Plan | `KUAE_API_KEY` | `https://coding-plan-endpoint.kuaecloud.net/v1` |
| `lambda_ai` | Lambda AI | `LAMBDA_API_KEY` | `https://api.lambda.ai/v1` |
| `lemonade` | Lemonade | `LEMONADE_API_KEY` | `http://localhost:13305/v1` |
| `lemonfox_ai` | Lemonfox AI | `LEMONFOX_API_KEY` | `https://api.lemonfox.ai/v1` |
| `libertai` | Libertai | `LIBERTAI_API_KEY` | `https://api.libertai.io/v1` |
| `lilac` | Lilac | `LILAC_API_KEY` | `https://api.getlilac.com/v1` |
| `lingyiwanwu` | Lingyiwanwu (零一万物) | `LINGYIWANWU_API_KEY` | `https://api.lingyiwanwu.com/v1` |
| `llama` | Llama | `LLAMA_API_KEY` | `https://api.llama.com/compat/v1/` |
| `llamagate` | Llamagate | `LLAMAGATE_API_KEY` | `https://api.llamagate.dev/v1` |
| `llmgateway` | LLM Gateway | `LLM_GATEWAY_API_KEY` | `https://api.llmgateway.io/v1` |
| `llmtr` | LLMTR | `LLMTR_API_KEY` | `https://llmtr.com/v1` |
| `longcat` | LongCat | `LONGCAT_API_KEY` | `https://api.longcat.chat/v1` |
| `lucidquery` | LucidQuery | `LUCIDQUERY_API_KEY` | `https://api.lucidquery.com/v1` |
| `lynkr` | Lynkr | `LYNKR_API_KEY` | `http://localhost:8081/v1` |
| `matterai` | Matter AI | `MATTERAI_API_KEY` | `https://api.matterai.com/v1` |
| `meganova` | Meganova | `MEGANOVA_API_KEY` | `https://api.meganova.ai/v1` |
| `merge_gateway` | Merge Gateway | `MERGE_GATEWAY_API_KEY` | `https://api-gateway.merge.dev/v1/openai` |
| `meta` | Meta | `MODEL_API_KEY` | `https://api.meta.ai/v1` |
| `meta_llama` | Meta Llama API | `LLAMA_API_KEY` | `https://api.llama.com/compat/v1` |
| `mimo` | Mimo | `MIMO_API_KEY` | `https://api.xiaomimimo.com/v1` |
| `minimax` | MiniMax | `MINIMAX_API_KEY` | `https://api.minimax.io/v1` |
| `minimax_cn` | MiniMax (minimaxi.com) | `MINIMAX_API_KEY` | `https://api.minimaxi.com/v1` |
| `minimax_cn_coding_plan` | MiniMax Token Plan (minimaxi.com) | `MINIMAX_API_KEY` | `https://api.minimaxi.com/v1` |
| `minimax_coding_plan` | MiniMax Token Plan (minimax.io) | `MINIMAX_API_KEY` | `https://api.minimax.io/anthropic/v1` |
| `mira` | Mira | `MIRA_API_KEY` | `https://api.mira.so/v1` |
| `mixlayer` | Mixlayer | `MIXLAYER_API_KEY` | `https://models.mixlayer.ai/v1` |
| `moark` | Moark | `MOARK_API_KEY` | `https://api.moark.com/v1` |
| `modal` | Modal | `MODAL_API_KEY` | `https://modal.com/v1` |
| `model_oracle_ai` | Model Oracle AI | `MODEL_ORACLE_API_KEY` | `https://api.modeloracle.com/api/v1` |
| `modelscope` | ModelScope | `MODELSCOPE_API_KEY` | `https://api-inference.modelscope.cn/v1` |
| `moonshotai` | Moonshot AI | `MOONSHOT_API_KEY` | `https://api.moonshot.cn/v1` |
| `moonshotai_cn` | Moonshot AI (China) | `MOONSHOT_API_KEY` | `https://api.moonshot.cn/anthropic/v1` |
| `morph` | Morph LLM | `MORPH_API_KEY` | `https://api.morphllm.com/v1` |
| `nanogpt` | NanoGPT | `NANOGPT_API_KEY` | `https://api.nanogpt.com/v1` |
| `ncompass` | Ncompass | `NCOMPASS_API_KEY` | `https://api.ncompass.tech/v1` |
| `nearai` | NEAR AI Cloud | `NEARAI_API_KEY` | `https://cloud-api.near.ai/v1` |
| `nebius` | Nebius AI | `NEBIUS_API_KEY` | `https://api.studio.nebius.ai/v1` |
| `neon` | Neon | `NEON_AI_GATEWAY_TOKEN` | `https://<branch-host>/v1` |
| `neuralwatt` | Neuralwatt | `NEURALWATT_API_KEY` | `https://api.neuralwatt.com/v1` |
| `nextbit` | NextBit | `NEXTBIT_API_KEY` | `https://api.nextbit.ai/v1` |
| `nlp_cloud` | NLP Cloud | `NLPCLOUD_API_KEY` | `https://api.nlpcloud.io/v1` |
| `nous_research` | Nous Research | `NOUS_API_KEY` | `https://api.nousresearch.com/v1` |
| `novita` | Novita AI | `NOVITA_API_KEY` | `https://api.novita.ai/v1` |
| `nscale` | Nscale | `NSCALE_API_KEY` | `https://inference.api.nscale.com/v1` |
| `nvidia_nim` | NVIDIA NIM | `NVIDIA_API_KEY` | `https://integrate.api.nvidia.com/v1` |
| `oci` | OCI | `OCI_API_KEY` | `https://inference.generativeai.${region}.oci.oraclecloud.com/openai/v1` |
| `ofox` | OfoxAI | `OFOX_API_KEY` | `https://api.ofox.ai/v1` |
| `ohmygpt` | OhMyGPT | `OHMYGPT_API_KEY` | `https://api.ohmygpt.com/v1` |
| `ollama_cloud` | Ollama Cloud | `OLLAMA_CLOUD_API_KEY` | `https://api.ollama.com/v1` |
| `openaimax` | OpenAIMax | `OPENAIMAX_API_KEY` | `https://api.openaimax.com/v1` |
| `openaisb` | OpenAI-SB | `OPENAISB_API_KEY` | `https://api.openaisb.com/v1` |
| `opencode` | OpenCode Zen | `OPENCODE_API_KEY` | `https://api.opencode.zen/v1` |
| `opencode_go` | OpenCode Go | `OPENCODE_GO_API_KEY` | `https://api.opencode.dev/v1` |
| `opencode_zen` | OpenCode Zen | `OPENCODE_ZEN_API_KEY` | `https://api.opencode.zen/v1` |
| `orcarouter` | OrcaRouter | `ORCAROUTER_API_KEY` | `https://api.orcarouter.com/v1` |
| `ovhcloud` | OVHcloud AI | `OVHCLOUD_API_KEY` | `https://oai.endpoints.kepler.ai.cloud.ovh.net/v1` |
| `parasail` | Parasail | `PARASAIL_API_KEY` | `https://api.parasail.io/v1` |
| `perfxcloud` | PerfXCloud | `PERFXCLOUD_API_KEY` | `https://api.perfxcloud.com/v1` |
| `perplexity` | Perplexity | `PERPLEXITY_API_KEY` | `https://api.perplexity.ai` |
| `perplexity_agent` | Perplexity Agent | `PERPLEXITY_API_KEY` | `https://api.perplexity.ai/v1` |
| `petals` | Petals | `PETALS_API_KEY` | `https://api.petals.dev/v1` |
| `pinstripes` | Pinstripes | `PINSTRIPES_API_KEY` | `https://api.pinstripes.io/v1` |
| `pioneer` | Pioneer | `PIONEER_API_KEY` | `https://api.pioneer.ai/v1` |
| `poe` | Poe | `POE_API_KEY` | `https://api.poe.com/v1` |
| `poolside` | Poolside | `POOLSIDE_API_KEY` | `https://inference.poolside.ai/v1` |
| `portkey` | Portkey Gateway | `PORTKEY_API_KEY` | `https://api.portkey.ai/v1` |
| `ppinfra` | PPInfra（PPIO 派欧云） | `PPIO_API_KEY` | `https://api.ppio.com/openai` |
| `predibase` | Predibase | `PREDIBASE_API_KEY` | `https://serving.app.predibase.com/v1` |
| `privatemode_ai` | Privatemode AI | `PRIVATEMODE_API_KEY` | `http://localhost:8080/v1` |
| `publicai` | Publicai | `PUBLICAI_API_KEY` | `https://platform.publicai.co/v1` |
| `qihang_ai` | QiHang（启航 AI） | `QIHANG_API_KEY` | `https://api.qhaigc.net/v1` |
| `qihoo360` | 360 AI | `AI360_API_KEY` | `https://api.360.cn/v1` |
| `qiniu_ai` | Qiniu AI | `QINIU_API_KEY` | `https://api.qiniu.com/v1` |
| `regolo_ai` | Regolo AI | `REGOLO_API_KEY` | `https://api.regolo.ai/v1` |
| `reka_ai` | Reka AI | `REKA_API_KEY` | `https://api.reka.ai/v1` |
| `requesty` | Requesty | `REQUESTY_API_KEY` | `https://api.requesty.ai/v1` |
| `reve` | Reve | `REVE_API_KEY` | `https://api.reve.ai/v1` |
| `routing_run` | routing.run | `ROUTING_RUN_API_KEY` | `https://api.routing.run/v1` |
| `sakana` | Sakana AI | `SAKANA_API_KEY` | `https://api.sakana.ai/v1` |
| `sambanova` | SambaNova | `SAMBANOVA_API_KEY` | `https://api.sambanova.ai/v1` |
| `sarvam` | Sarvam AI | `SARVAM_API_KEY` | `https://api.sarvam.ai/v1` |
| `scaleway` | Scaleway AI | `SCALEWAY_API_KEY` | `https://api.scaleway.ai/v1` |
| `scx_ai` | SCX AI | `SCX_AI_API_KEY` | `https://api.scx.ai/v1` |
| `siliconflow` | SiliconFlow | `SILICONFLOW_API_KEY` | `https://api.siliconflow.cn/v1` |
| `snowflake` | Snowflake | `SNOWFLAKE_PAT` | `https://<account-identifier>.snowflakecomputing.com/api/v2/cortex/v1` |
| `snowflake_cortex` | Snowflake Cortex | `SNOWFLAKE_CORTEX_PAT` | `https://<account-identifier>.snowflakecomputing.com/api/v2/cortex/v1` |
| `stackit` | STACKIT | `STACKIT_API_KEY` | `https://api.openai-compat.model-serving.eu01.onstackit.cloud/v1` |
| `stepfun` | StepFun (阶跃星辰) | `STEPFUN_API_KEY` | `https://api.stepfun.com/v1` |
| `stepfun_ai_step_plan` | StepFun Step Plan (Global) | `STEPFUN_API_KEY` | `https://api.stepfun.ai/step_plan/v1` |
| `stepfun_step_plan` | StepFun Step Plan (China) | `STEPFUN_API_KEY` | `https://api.stepfun.com/step_plan/v1` |
| `subconscious` | Subconscious | `SUBCONSCIOUS_API_KEY` | `https://api.subconscious.dev/v1` |
| `submodel` | SubModel | `SUBMODEL_API_KEY` | `https://api.submodel.com/v1` |
| `synthetic` | Synthetic | `SYNTHETIC_API_KEY` | `https://api.synthetic.new/openai/v1` |
| `tencent` | Tencent (混元/Hunyuan) | `TENCENT_API_KEY` | `https://api.hunyuan.cloud.tencent.com/v1` |
| `tencent_coding_plan` | Tencent Coding Plan (China) | `TENCENT_CODING_PLAN_API_KEY` | `https://api.lkeap.cloud.tencent.com/coding/v3` |
| `tencent_token_plan` | Tencent Token Plan | `TENCENT_TOKEN_PLAN_API_KEY` | `https://api.lkeap.cloud.tencent.com/plan/v3` |
| `tencent_token_plan_enterprise_auto` | 腾讯云 Token Plan / Token Plan 企业版轻享套餐 | `TENCENT_TOKEN_PLAN_ENTERPRISE_API_KEY` | `https://tokenhub.tencentmaas.com/plan/v3` |
| `tencent_token_plan_enterprise_pro` | 腾讯云 Token Plan / Token Plan 企业版专业套餐 | `TENCENT_TOKEN_PLAN_ENTERPRISE_API_KEY` | `https://tokenhub.tencentmaas.com/plan/v3` |
| `tencent_token_plan_general_personal` | 腾讯云 Token Plan / 通用 Token Plan（个人版） | `TENCENT_TOKEN_PLAN_API_KEY` | `https://api.lkeap.cloud.tencent.com/plan/v3` |
| `tencent_token_plan_hy_personal` | 腾讯云 Token Plan / Hy Token Plan（个人版） | `TENCENT_TOKEN_PLAN_API_KEY` | `https://api.lkeap.cloud.tencent.com/plan/v3` |
| `tencent_tokenhub` | Tencent TokenHub | `TENCENT_TOKENHUB_API_KEY` | `https://tokenhub.tencentmaas.com/v1` |
| `tensormesh` | Tensormesh | `YOUR_API_KEY` | `https://serverless.tensormesh.ai` |
| `the_grid_ai` | The Grid AI | `THEGRIDAI_API_KEY` | `https://api.thegrid.ai/v1` |
| `thinkingmachines` | Thinking Machines | `TINKER_API_KEY` | `https://tinker.thinkingmachines.dev/services/tinker-prod/oai/api/v1` |
| `tinfoil` | Tinfoil | `TINFOIL_API_KEY` | `https://inference.tinfoil.sh/v1` |
| `togetherai` | Together AI | `TOGETHER_API_KEY` | `https://api.together.xyz/v1` |
| `tokenflux` | Tokenflux | `TOKENFLUX_API_KEY` | `https://tokenflux.ai/v1` |
| `tokenpony` | TokenPony | `TOKENPONY_API_KEY` | `https://api.tokenpony.com/v1` |
| `trustedrouter` | TrustedRouter | `TRUSTEDROUTER_API_KEY` | `https://api.trustedrouter.com/v1` |
| `tundra` | Tundra | `TUNDRA_API_KEY` | `https://api.tundra.ai/v1` |
| `umans_ai` | Umans AI | `UMANS_AI_API_KEY` | `https://api.code.umans.ai/v1` |
| `unorouter` | UnoRouter | `UNOROUTER_API_KEY` | `https://unorouter.com/en` |
| `upstage` | Upstage | `UPSTAGE_API_KEY` | `https://api.upstage.ai/v1` |
| `v0` | v0 (Vercel) | `V0_API_KEY` | `https://api.v0.dev/v1` |
| `venice` | Venice | `VENICE_API_KEY` | `https://api.venice.ai/api/v1` |
| `vercel` | Vercel | `VERCEL_API_KEY` | `https://api.v0.dev/v1` |
| `vivgrid` | Vivgrid | `VIVGRID_API_KEY` | `https://api.vivgrid.com/v1` |
| `volc_engine` | VolcEngine | `ARK_API_KEY` | `https://ark.cn-beijing.volces.com/api/v3` |
| `vultr` | Vultr | `VULTR_API_KEY` | `https://api.vultrinference.com/v1` |
| `wafer` | Wafer | `WAFER_API_KEY` | `https://api.wafer.ai/v1` |
| `wandb` | Weights & Biases | `WANDB_API_KEY` | `https://api.inference.wandb.ai/v1` |
| `xiaomi_token_plan_ams` | Xiaomi Token Plan (Europe) | `MIMO_API_KEY` | `https://token-plan-ams.xiaomimimo.com/v1` |
| `xiaomi_token_plan_cn` | Xiaomi Token Plan (China) | `MIMO_API_KEY` | `https://token-plan-cn.xiaomimimo.com/v1` |
| `xiaomi_token_plan_sgp` | Xiaomi Token Plan (Singapore) | `MIMO_API_KEY` | `https://token-plan-sgp.xiaomimimo.com/v1` |
| `xiaomimimo` | Xiaomi MiMo | `XIAOMI_API_KEY` | `https://mimo.xiaomi.com/v1` |
| `xpersona` | Xpersona | `XPERSONA_API_KEY` | `https://www.xpersona.co/v1` |
| `xunfei` | Xunfei | `XUNFEI_API_PASSWORD` | `https://spark-api-open.xf-yun.com/v1` |
| `zai` | Zai | `ZAI_API_KEY` | `https://api.z.ai/api/paas/v4` |
| `zai_coding_plan` | Z.AI Coding Plan | `ZHIPU_API_KEY` | `https://api.z.ai/api/anthropic` |
| `zeldoc` | Zeldoc | `ZELDOC_API_KEY` | `https://api.zeldoc.ai/v1` |
| `zenmux` | ZenMux | `ZENMUX_API_KEY` | `https://zenmux.ai/api/v1` |
| `zhipu_v4` | ZhipuV4 | `ZHIPU_API_KEY` | `https://open.bigmodel.cn/api/paas/v4` |
| `zhipuai_coding_plan` | Zhipu AI Coding Plan | `ZHIPU_API_KEY` | `https://open.bigmodel.cn/api/coding/paas/v4` |

## Typed factories (non-registry)

These providers are **not** name-addressable: `provider("anthropic", ...)` fails with `UnknownProvider`. Use the typed entry points below (Rust type names; per-binding constructors: see [reference.md](reference.md)).

### Native protocol providers

| module | typed entry points |
|--------|--------------------|
| `replay` | — |
| `catalogue` | `Catalogue` |
| `anthropic` | `AnthropicConfig` / `AnthropicProvider` |
| `anthropic_aws` | `AnthropicAwsConfig` / `AnthropicAwsProvider` |
| `azure` | `AzureConfig` / `AzureProvider` |
| `bedrock` | `BedrockConfig` / `BedrockProvider` |
| `cohere` | `CohereConfig` / `CohereProvider` |
| `google` | `GoogleConfig` / `GoogleProvider` |
| `mistral` | `MistralConfig` / `MistralProvider` |
| `openai` | `OpenAIConfig` / `OpenAIProvider` |
| `vertex` | `VertexConfig` / `VertexProvider` |
| `voyage` | `VoyageConfig` / `VoyageProvider` |
| `codex` | `CodexConfig` / `CodexProvider` |
| `openrouter` | `OpenRouterConfig` / `OpenRouterProvider` |
| `xai` | `XAIConfig` / `XAIProvider` |

### OpenAI-compatible thin wrappers (second batch)

| module | typed entry points |
|--------|--------------------|
| `huggingface` | `HuggingFaceConfig` / `HuggingFaceProvider` |
| `llamafile` | `LlamafileConfig` / `LlamafileProvider` |
| `lmstudio` | `LmStudioConfig` / `LmStudioProvider` |
| `mistralrs` | `MistralrsConfig` / `MistralrsProvider` |
| `ollama` | `OllamaConfig` / `OllamaProvider` |

### Speech-only providers (TTS)

| module | typed entry points |
|--------|--------------------|
| `cartesia` | `CartesiaConfig` / `CartesiaProvider` |
| `elevenlabs` | `ElevenLabsConfig` / `ElevenLabsProvider` |
| `hume` | `HumeConfig` / `HumeProvider` |
| `lmnt` | `LMNTConfig` / `LMNTProvider` |

### Transcription-only providers (STT)

| module | typed entry points |
|--------|--------------------|
| `assemblyai` | `AssemblyAIConfig` / `AssemblyAIProvider` |
| `deepgram` | `DeepgramConfig` / `DeepgramProvider` |
| `fal` | `FalConfig` / `FalProvider` |
| `gladia` | `GladiaConfig` / `GladiaProvider` |
| `revai` | `RevaiConfig` / `RevaiProvider` |

### Image-only providers

| module | typed entry points |
|--------|--------------------|
| `black_forest_labs` | `BlackForestLabsConfig` / `BlackForestLabsProvider` |
| `luma` | `LumaConfig` / `LumaProvider` |
| `prodia` | `ProdiaConfig` / `ProdiaProvider` |
| `replicate` | `ReplicateConfig` / `ReplicateProvider` |

### Video-only providers

| module | typed entry points |
|--------|--------------------|
| `klingai` | `KlingAIConfig` / `KlingAIProvider` |

### Generic Responses API wrapper

| module | typed entry points |
|--------|--------------------|
| `open_responses` | `OpenResponsesConfig` / `OpenResponsesProvider` |

### Bulk-generated thin-wrapper providers

| module | typed entry points |
|--------|--------------------|
| `cybertron` | `CybertronConfig` / `CybertronProvider` |
| `docker_model_runner` | `DockerModelRunnerConfig` / `DockerModelRunnerProvider` |
| `gaudi` | `GaudiConfig` / `GaudiProvider` |
| `jlama` | `JlamaConfig` / `JlamaProvider` |
| `litellm_proxy` | `LitellmProxyConfig` / `LitellmProxyProvider` |
| `llamacpp` | `LlamacppConfig` / `LlamacppProvider` |
| `local` | `LocalConfig` / `LocalProvider` |
| `localai` | `LocalaiConfig` / `LocalaiProvider` |
| `mlx` | `MlxConfig` / `MlxProvider` |
| `omlx` | `OmlxConfig` / `OmlxProvider` |
| `onnx` | `OnnxConfig` / `OnnxProvider` |
| `oobabooba` | `OobaboobaConfig` / `OobaboobaProvider` |
| `openvino` | `OpenvinoConfig` / `OpenvinoProvider` |
| `sglang` | `SglangConfig` / `SglangProvider` |
| `vllm` | `VllmConfig` / `VllmProvider` |
| `xinference` | `XinferenceConfig` / `XinferenceProvider` |

### Modality-specific providers (non-language, e.g. rerank-only)

| module | typed entry points |
|--------|--------------------|
| `jina_ai` | `JinaAiConfig` / `JinaAiProvider` |

### AWS Polly speech (TTS) provider — SigV4 authenticated, speech modality only

| module | typed entry points |
|--------|--------------------|
| `aws_polly` | `AwsPollyConfig` / `AwsPollyProvider` |

### Recraft image provider (OpenAI Images-compatible + Recraft extension fields)

| module | typed entry points |
|--------|--------------------|
| `recraft` | `RecraftConfig` / `RecraftProvider` |

### Stability image provider (image modality only)

| module | typed entry points |
|--------|--------------------|
| `stability` | `StabilityConfig` / `StabilityProvider` |

### Video-only provider (runwayml)

| module | typed entry points |
|--------|--------------------|
| `runwayml` | `RunwaymlConfig` / `RunwaymlProvider` |

### P1 thin-wrapper providers (provider-research batch)

| module | typed entry points |
|--------|--------------------|
| `bedrock_mantle` | `BedrockMantleConfig` / `BedrockMantleProvider` |

### Vertex AI MaaS partner-model providers (OpenAI-compatible thin wrappers). Each wraps the shared OpenAIProvider against the Vertex AI MaaS OpenAPI endpoint, authenticating with a Google Cloud Bearer token

| module | typed entry points |
|--------|--------------------|
| `vertex_ai_ai21_models` | `VertexAiAi21ModelsConfig` / `VertexAiAi21ModelsProvider` |
| `vertex_ai_anthropic_models` | `VertexAiAnthropicModelsConfig` / `VertexAiAnthropicModelsProvider` |
| `vertex_ai_deepseek_models` | `VertexAiDeepseekModelsConfig` / `VertexAiDeepseekModelsProvider` |
| `vertex_ai_llama_models` | `VertexAiLlamaModelsConfig` / `VertexAiLlamaModelsProvider` |
| `vertex_ai_minimax_models` | `VertexAiMinimaxModelsConfig` / `VertexAiMinimaxModelsProvider` |
| `vertex_ai_mistral_models` | `VertexAiMistralModelsConfig` / `VertexAiMistralModelsProvider` |
| `vertex_ai_moonshot_models` | `VertexAiMoonshotModelsConfig` / `VertexAiMoonshotModelsProvider` |
| `vertex_ai_openai_models` | `VertexAiOpenaiModelsConfig` / `VertexAiOpenaiModelsProvider` |
| `vertex_ai_qwen_models` | `VertexAiQwenModelsConfig` / `VertexAiQwenModelsProvider` |
| `vertex_ai_zai_models` | `VertexAiZaiModelsConfig` / `VertexAiZaiModelsProvider` |

### Search-only providers (web search modality)

| module | typed entry points |
|--------|--------------------|
| `dataforseo` | `DataforseoConfig` / `DataforseoProvider` |
| `exa_ai` | `ExaAiConfig` / `ExaAiProvider` |
| `firecrawl` | `FirecrawlConfig` / `FirecrawlProvider` |
| `google_pse` | `GooglePseConfig` / `GooglePseProvider` |
| `linkup` | `LinkupConfig` / `LinkupProvider` |
| `parallel_ai` | `ParallelAiConfig` / `ParallelAiProvider` |
| `searxng` | `SearxngConfig` / `SearxngProvider` |
| `serper` | `SerperConfig` / `SerperProvider` |
| `tavily` | `TavilyConfig` / `TavilyProvider` |
| `tinyfish` | `TinyfishConfig` / `TinyfishProvider` |
| `you_com` | `YouComConfig` / `YouComProvider` |
