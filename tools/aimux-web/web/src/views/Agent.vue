<script setup lang="ts">
import { reactive, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { useAppStore } from '../stores/app'
import { runAgent, newRun, type AgentDef, type AgentRun, type AgentStepView } from '../agent/engine'
import StepCard from '../components/StepCard.vue'
import Button from '../components/ui/Button.vue'
import Input from '../components/ui/Input.vue'
import Textarea from '../components/ui/Textarea.vue'
import Combobox from '../components/ui/Combobox.vue'
import Slider from '../components/ui/Slider.vue'
import Label from '../components/ui/Label.vue'
import Badge from '../components/ui/Badge.vue'
import Separator from '../components/ui/Separator.vue'
import Switch from '../components/ui/Switch.vue'

const store = useAppStore()
const router = useRouter()

const LS_KEY = 'aimux-web:agent-def'

function defaultDef(): AgentDef {
  return {
    name: 'calculator-agent',
    system_prompt:
      'You are a helpful assistant. Always use the calculator tool for arithmetic, then answer.',
    provider: 'openai',
    model: 'gpt-4o',
    api_key: '',
    base_url: '',
    tools: ['calculator', 'datetime'],
    max_steps: 8,
    temperature: 0,
    session_id: store.newSessionId(),
    mock: false,
  }
}

function loadDef(): AgentDef | null {
  try {
    const raw = localStorage.getItem(LS_KEY)
    return raw ? { ...defaultDef(), ...JSON.parse(raw) } : null
  } catch {
    return null
  }
}

const agentDef = ref<AgentDef>(loadDef() ?? defaultDef())
watch(agentDef, (d) => localStorage.setItem(LS_KEY, JSON.stringify(d)), { deep: true })

const run = ref<AgentRun>(newRun())
const userText = ref('')
const history = ref<Array<{ user: string; steps: AgentStepView[]; status: AgentRun['status'] }>>([])
const error = ref<string | null>(null)

const toolMap = () => new Map(store.tools.map((t) => [t.name, t]))

function newSession() {
  agentDef.value.session_id = store.newSessionId()
}

async function go() {
  const text = userText.value.trim()
  if (!text || run.value.status === 'running') return
  userText.value = ''
  error.value = null
  await runAgent(agentDef.value, run.value, toolMap(), text)
  history.value.push({ user: text, steps: run.value.steps, status: run.value.status })
  run.value.steps = []
  run.value.status = 'idle'
}

function stop() {
  run.value.abort()
}

function reset() {
  run.value.abort()
  history.value = []
  run.value.steps = []
  run.value.status = 'idle'
  error.value = null
}

function exportDef() {
  const blob = new Blob([JSON.stringify(agentDef.value, null, 2)], { type: 'application/json' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = `${agentDef.value.name || 'agent'}.json`
  a.click()
  URL.revokeObjectURL(url)
}

function openTrace(step: AgentStepView) {
  if (step.meta?.call_id) router.push({ path: '/traces', query: { call: step.meta.call_id } })
}

type BadgeVariant = 'default' | 'secondary' | 'success' | 'warning' | 'destructive'
function statusBadge(status: string): BadgeVariant {
  if (status === 'done') return 'success'
  if (status === 'error') return 'destructive'
  if (status === 'running') return 'warning'
  return 'secondary'
}
</script>

<template>
  <div class="flex h-full min-w-0">
    <!-- conversation -->
    <div class="flex min-w-0 flex-1 flex-col">
      <div class="flex items-center gap-2 border-b px-4 py-2">
        <span class="text-sm font-semibold">Agent · {{ agentDef.name }}</span>
        <Badge variant="secondary" class="font-mono text-[10px]">{{ agentDef.provider }}/{{ agentDef.model }}</Badge>
        <div class="flex-1" />
        <Button variant="outline" size="sm" @click="reset">重置</Button>
      </div>

      <div class="flex-1 overflow-auto p-4">
        <div v-if="!history.length && !run.steps.length" class="flex h-full items-center justify-center text-sm text-muted-foreground">
          Agent loop 在浏览器里跑（RFC-0029 决策 D1）：每次 model call 走后端，多步用同一个 session_id 归组。发消息开始。
        </div>

        <div v-for="(rec, hi) in history" :key="hi" class="mb-6">
          <div class="mb-2 flex items-start gap-3">
            <Badge variant="secondary" class="mt-1 w-16 shrink-0 justify-center">user</Badge>
            <div class="whitespace-pre-wrap text-sm leading-relaxed">{{ rec.user }}</div>
          </div>
          <div v-for="step in rec.steps" :key="`h${hi}-s${step.step}`" class="ml-[4.75rem]">
            <StepCard :step="step" @trace="openTrace(step)" />
          </div>
        </div>

        <template v-if="run.steps.length">
          <div class="mb-2 flex items-start gap-3">
            <Badge variant="secondary" class="mt-1 w-16 shrink-0 justify-center">user</Badge>
            <div class="whitespace-pre-wrap text-sm leading-relaxed">{{ history.length ? history[history.length - 1].user : '…' }}</div>
          </div>
          <div v-for="step in run.steps" :key="`live-s${step.step}`" class="ml-[4.75rem]">
            <StepCard :step="step" @trace="openTrace(step)" />
          </div>
        </template>
      </div>

      <div class="border-t p-3">
        <div class="flex items-end gap-2">
          <Textarea
            v-model="userText"
            :rows="2"
            placeholder="给 agent 一个任务，例如：17 * 19 = ?"
            class="flex-1"
            @keydown.enter.exact.prevent="go"
          />
          <Button v-if="run.status !== 'running'" variant="default" @click="go">运行</Button>
          <Button v-else variant="destructive" @click="stop">停止</Button>
        </div>
        <div v-if="error" class="mt-2 text-xs text-destructive">{{ error }}</div>
      </div>
    </div>

    <!-- agent definition + timeline -->
    <aside class="flex w-80 shrink-0 flex-col overflow-auto border-l">
      <div class="space-y-4 p-4">
        <div>
          <Label>名称</Label>
          <Input v-model="agentDef.name" class="mt-1" />
        </div>
        <div>
          <Label>System prompt</Label>
          <Textarea v-model="agentDef.system_prompt" :rows="5" class="mt-1" />
        </div>
        <div>
          <Label>Provider</Label>
          <Combobox v-model="agentDef.provider" :options="store.providers" class="mt-1" placeholder="搜索 provider…" />
        </div>
        <div>
          <Label>Model</Label>
          <Input v-model="agentDef.model" class="mt-1 font-mono" placeholder="model id" />
        </div>
        <div>
          <Label>API key（env 引用）</Label>
          <Input v-model="agentDef.api_key" class="mt-1 font-mono" placeholder="env:OPENAI_API_KEY" />
        </div>
        <div>
          <Label>Tools</Label>
          <div class="mt-1 space-y-1">
            <label v-for="t in store.tools" :key="t.name" class="flex cursor-pointer items-center gap-2 text-sm">
              <input v-model="agentDef.tools" type="checkbox" :value="t.name" class="accent-foreground" />
              <span>{{ t.name }}</span>
            </label>
          </div>
        </div>
        <div class="flex items-center justify-between">
          <Label>Max steps <span class="text-muted-foreground">{{ agentDef.max_steps }}</span></Label>
          <Slider v-model="agentDef.max_steps" :min="1" :max="20" :step="1" class="ml-3 w-32" />
        </div>
        <div class="flex items-center justify-between">
          <Label>Temperature <span class="text-muted-foreground">{{ agentDef.temperature.toFixed(1) }}</span></Label>
          <Slider v-model="agentDef.temperature" :min="0" :max="2" :step="0.1" class="ml-3 w-32" />
        </div>
        <div class="flex items-center justify-between">
          <Label>Mock 模式（离线）</Label>
          <Switch v-model="agentDef.mock" />
        </div>

        <Separator />

        <div class="flex gap-2">
          <Button variant="outline" size="sm" class="flex-1" @click="newSession">新会话</Button>
          <Button variant="outline" size="sm" class="flex-1" @click="exportDef">导出定义</Button>
        </div>
      </div>

      <div class="border-t p-4">
        <div class="mb-2 text-xs font-semibold text-muted-foreground">步骤时间线</div>
        <div class="space-y-1.5">
          <template v-if="history.length">
            <div v-for="(rec, hi) in history" :key="`t${hi}`">
              <div class="mb-1 text-[11px] text-muted-foreground">#{{ hi + 1 }} {{ rec.user.slice(0, 20) }}</div>
              <div v-for="s in rec.steps" :key="`ts${hi}-${s.step}`" class="flex cursor-pointer items-center gap-2 rounded px-2 py-1 text-[11px] hover:bg-accent/40" @click="openTrace(s)">
                <span class="w-12 text-muted-foreground">step{{ s.step }}</span>
                <Badge :variant="statusBadge(s.status)" class="w-10 justify-center">{{ s.status }}</Badge>
                <span class="text-muted-foreground">{{ s.toolCalls.length ? `${s.toolCalls.length} tool` : '答复' }}</span>
              </div>
            </div>
          </template>
          <div v-if="run.steps.length" class="space-y-1">
            <div v-for="s in run.steps" :key="`tl${s.step}`" class="flex items-center gap-2 rounded px-2 py-1 text-[11px]">
              <span class="w-12 text-muted-foreground">step{{ s.step }}</span>
              <Badge variant="warning" class="w-10 justify-center">{{ s.status }}</Badge>
              <span class="animate-pulse text-muted-foreground">运行中…</span>
            </div>
          </div>
          <div v-if="!history.length && !run.steps.length" class="text-xs text-muted-foreground">（尚无运行）</div>
        </div>
      </div>
    </aside>
  </div>
</template>
