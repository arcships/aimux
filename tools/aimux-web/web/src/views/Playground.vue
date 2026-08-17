<script setup lang="ts">
import { computed, ref } from 'vue'
import { RouterLink, useRouter } from 'vue-router'
import { useAppStore } from '../stores/app'
import { api, callStream, parseStreamPart } from '../api/client'
import type { WireMessage } from '../types/WireMessage'
import type { WireContentPart } from '../types/WireContentPart'
import type { WireCallRequest } from '../types/WireCallRequest'
import Button from '../components/ui/Button.vue'
import Input from '../components/ui/Input.vue'
import Textarea from '../components/ui/Textarea.vue'
import Combobox from '../components/ui/Combobox.vue'
import Select from '../components/ui/Select.vue'
import Slider from '../components/ui/Slider.vue'
import Switch from '../components/ui/Switch.vue'
import Label from '../components/ui/Label.vue'
import Card from '../components/ui/Card.vue'
import Badge from '../components/ui/Badge.vue'
import Separator from '../components/ui/Separator.vue'
import StreamMessage from '../components/StreamMessage.vue'

const store = useAppStore()
const router = useRouter()

// ── model / params ──────────────────────────────────────────────────────────
const provider = ref('openai')
const model = ref('gpt-4o')
const apiKey = ref('')
const baseUrl = ref('')
const stream = ref(true)
const mock = ref(false)
const temperature = ref(0.7)
const maxTokens = ref(1024)
const selectedTools = ref<string[]>([])
const responseFormat = ref('text')
const headersJson = ref('{}')
const overridesJson = ref('{}')

const suggested = computed(() => store.suggestedModels[provider.value] ?? [])
const modelOptions = computed(() =>
  suggested.value.length ? [...suggested.value, '__custom__'] : ['__custom__'],
)
const modelCustom = ref(false)

const apiKeyHint = computed(() => {
  const envVar = providerEnvVar(provider.value)
  return envVar ? `env:${envVar}` : 'env:VAR'
})

function providerEnvVar(p: string): string | null {
  const map: Record<string, string> = {
    openai: 'OPENAI_API_KEY',
    anthropic: 'ANTHROPIC_API_KEY',
    google: 'GOOGLE_GENERATIVE_AI_API_KEY',
    mistral: 'MISTRAL_API_KEY',
    xai: 'XAI_API_KEY',
    cohere: 'COHERE_API_KEY',
  }
  return map[p] ?? null
}

function onModelPick(v: string) {
  if (v === '__custom__') {
    modelCustom.value = true
  } else {
    model.value = v
    modelCustom.value = false
  }
}

// ── chat ────────────────────────────────────────────────────────────────────
interface ChatItem {
  role: 'user' | 'assistant'
  text: string
  toolCalls: Array<{ id: string; name: string; input: unknown }>
  final: boolean
  meta?: { call_id?: string; session_id?: string | null; step?: number | null; outcome?: string }
  error?: string
}
const chat = ref<ChatItem[]>([])
const wireMessages = ref<WireMessage[]>([])
const inputText = ref('')
const running = ref(false)
const error = ref<string | null>(null)
const sessionId = ref(store.newSessionId())
let controller: AbortController | null = null

function resetConversation() {
  chat.value = []
  wireMessages.value = []
  sessionId.value = store.newSessionId()
  error.value = null
}

function wirePart(role: 'user' | 'assistant', item: ChatItem): WireMessage {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const content: any[] = []
  if (item.text.trim()) content.push({ type: 'text', text: item.text })
  for (const tc of item.toolCalls) {
    content.push({ type: 'tool_call', tool_call_id: tc.id, tool_name: tc.name, input: tc.input })
  }
  return { role, content } as WireMessage
}

async function send() {
  const text = inputText.value.trim()
  if (!text || running.value) return
  inputText.value = ''
  error.value = null

  const userItem: ChatItem = { role: 'user', text, toolCalls: [], final: true }
  chat.value.push(userItem)
  wireMessages.value.push({ role: 'user', content: [{ type: 'text', text }] })

  const assistant: ChatItem = { role: 'assistant', text: '', toolCalls: [], final: false }
  chat.value.push(assistant)
  running.value = true
  controller = new AbortController()

  const tools: unknown[] | null = selectedTools.value.length
    ? selectedTools.value.map((n) => store.tools.find((t) => t.name === n)).filter(Boolean)
    : null

  const body = {
    provider: provider.value,
    model: model.value,
    api_key: apiKey.value || null,
    base_url: baseUrl.value || null,
    stream: stream.value,
    mock: mock.value,
    options: {
      temperature: temperature.value,
      max_output_tokens: maxTokens.value,
      tools,
      response_format: responseFormat.value === 'json' ? { json: {} } : null,
      headers: safeJson(headersJson.value),
      body_overrides: safeJson(overridesJson.value),
    },
    session_id: sessionId.value,
    messages: wireMessages.value,
  } as unknown as WireCallRequest

  try {
    if (body.stream) {
      for await (const ev of callStream(body, controller.signal)) {
        if (ev.event === 'stream_part') {
          const part = parseStreamPart(ev.data)
          if ('TextDelta' in part) assistant.text += part.TextDelta.delta
          else if ('ToolCall' in part)
            assistant.toolCalls.push({ id: part.ToolCall.tool_call_id, name: part.ToolCall.tool_name, input: part.ToolCall.input })
        } else if (ev.event === 'meta') {
          assistant.meta = JSON.parse(ev.data)
        } else if (ev.event === 'error') {
          throw new Error(ev.data)
        }
      }
    } else {
      const res = await api.call(body)
      assistant.text = res.text
      if (res.meta) assistant.meta = res.meta
      if (res.error) throw new Error(res.error)
    }
    assistant.final = true
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    wireMessages.value.push(wirePart('assistant', assistant) as any)
  } catch (e) {
    if (controller.signal.aborted) {
      assistant.error = '已停止'
    } else {
      assistant.error = String(e)
      error.value = String(e)
    }
    assistant.final = true
  } finally {
    running.value = false
    controller = null
  }
}

function stop() {
  controller?.abort()
}

function safeJson(s: string): unknown | null {
  if (!s.trim()) return null
  try {
    return JSON.parse(s)
  } catch {
    return null
  }
}

function openTrace(item: ChatItem) {
  if (item.meta?.call_id) {
    router.push({ path: '/traces', query: { call: item.meta.call_id } })
  }
}
</script>

<template>
  <div class="flex h-full min-w-0">
    <!-- main chat -->
    <div class="flex min-w-0 flex-1 flex-col">
      <div class="flex items-center gap-2 border-b px-4 py-2">
        <span class="text-sm font-semibold">Playground</span>
        <Badge variant="secondary" class="font-mono text-[10px]">{{ provider }}/{{ model }}</Badge>
        <div class="flex-1" />
        <span class="text-[11px] text-muted-foreground">session: {{ sessionId.slice(0, 14) }}…</span>
        <Button variant="outline" size="sm" @click="resetConversation">清空</Button>
      </div>

      <div class="flex-1 overflow-auto p-4">
        <div v-if="!chat.length" class="flex h-full items-center justify-center text-sm text-muted-foreground">
          选好 provider/model，输入消息开始验证。tool 调用会显示在助手消息里（多轮循环请用 Agent 页）。
        </div>
        <div v-for="(item, i) in chat" :key="i" class="mb-4">
          <div class="flex items-start gap-3">
            <Badge :variant="item.role === 'user' ? 'secondary' : 'default'" class="mt-1 shrink-0 w-16 justify-center">
              {{ item.role }}
            </Badge>
            <div class="min-w-0 flex-1">
              <StreamMessage v-if="item.role === 'assistant'" :content="item.text" :final="item.final" />
              <div v-else class="whitespace-pre-wrap text-sm leading-relaxed">{{ item.text }}</div>

              <div v-if="item.toolCalls.length" class="mt-2 space-y-1">
                <div v-for="tc in item.toolCalls" :key="tc.id" class="rounded-md border bg-muted/50 px-3 py-2 font-mono text-xs">
                  <span class="font-semibold text-foreground">⟐ {{ tc.name }}</span>
                  <span class="ml-2 text-muted-foreground">{{ JSON.stringify(tc.input) }}</span>
                </div>
              </div>

              <div v-if="item.error" class="mt-1 text-xs text-destructive">{{ item.error }}</div>
              <button
                v-if="item.meta?.call_id"
                class="mt-1 text-xs text-muted-foreground underline decoration-dotted hover:text-foreground cursor-pointer"
                @click="openTrace(item)"
              >
                view trace →
              </button>
            </div>
          </div>
        </div>
      </div>

      <div class="border-t p-3">
        <div class="flex items-end gap-2">
          <Textarea
            v-model="inputText"
            :rows="2"
            placeholder="输入消息，Enter 发送（Shift+Enter 换行）…"
            class="flex-1"
            @keydown.enter.exact.prevent="send"
          />
          <Button v-if="!running" variant="default" @click="send">发送</Button>
          <Button v-else variant="destructive" @click="stop">停止</Button>
        </div>
        <div v-if="error" class="mt-2 text-xs text-destructive">{{ error }}</div>
      </div>
    </div>

    <!-- params -->
    <aside class="w-72 shrink-0 overflow-auto border-l p-4">
      <div class="space-y-4">
        <div>
          <Label>Provider</Label>
          <Combobox v-model="provider" :options="store.providers" class="mt-1" placeholder="搜索 provider…" />
        </div>
        <div>
          <Label>Model</Label>
          <Combobox v-if="!modelCustom" :model-value="model" :options="modelOptions" class="mt-1" placeholder="选择或自定义…" @update:model-value="onModelPick" />
          <Input v-else v-model="model" class="mt-1 font-mono" placeholder="自定义 model id" />
          <button class="mt-1 text-[11px] text-muted-foreground underline decoration-dotted cursor-pointer" @click="modelCustom = !modelCustom">
            {{ modelCustom ? '用预设' : '自定义 model id' }}
          </button>
        </div>
        <div>
          <Label>API key（env 引用；留空用 Settings）</Label>
          <Input v-model="apiKey" class="mt-1 font-mono" :placeholder="apiKeyHint" />
          <div class="mt-1 text-[11px] text-muted-foreground">
            也可在 <RouterLink to="/settings" class="underline decoration-dotted">Settings</RouterLink> 保存该 provider 的 key，此处留空即可。
          </div>
        </div>
        <div>
          <Label>Base URL（可选）</Label>
          <Input v-model="baseUrl" class="mt-1 font-mono" placeholder="https://… 代理/本地" />
        </div>

        <Separator />

        <div class="flex items-center justify-between">
          <Label>流式输出</Label>
          <Switch v-model="stream" />
        </div>
        <div class="flex items-center justify-between">
          <Label>Mock 模式（离线）</Label>
          <Switch v-model="mock" />
        </div>

        <div>
          <Label>Temperature <span class="text-muted-foreground">{{ temperature.toFixed(1) }}</span></Label>
          <Slider v-model="temperature" :min="0" :max="2" :step="0.1" class="mt-2" />
        </div>
        <div>
          <Label>Max output tokens</Label>
          <Input v-model="maxTokens" type="number" class="mt-1 font-mono" />
        </div>
        <div>
          <Label>Response format</Label>
          <Select v-model="responseFormat" class="mt-1" :options="[{label:'text',value:'text'},{label:'json',value:'json'}]" />
        </div>
        <div>
          <Label>Tools</Label>
          <div class="mt-1 space-y-1">
            <label v-for="t in store.tools" :key="t.name" class="flex cursor-pointer items-center gap-2 text-sm">
              <input
                v-model="selectedTools"
                type="checkbox"
                :value="t.name"
                class="accent-foreground"
              />
              <span>{{ t.name }}</span>
            </label>
            <div v-if="!store.tools.length" class="text-xs text-muted-foreground">加载中…</div>
          </div>
        </div>
        <div>
          <Label>Headers（JSON）</Label>
          <Textarea v-model="headersJson" :rows="2" class="mt-1" />
        </div>
        <div>
          <Label>Body overrides（JSON）</Label>
          <Textarea v-model="overridesJson" :rows="3" class="mt-1" />
        </div>
      </div>
    </aside>
  </div>
</template>
