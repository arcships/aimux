<script setup lang="ts">
import { onMounted, ref, shallowRef } from 'vue'
import { api } from '../api/client'
import type { Recording } from '../types/Recording'
import Button from '../components/ui/Button.vue'
import Input from '../components/ui/Input.vue'
import Combobox from '../components/ui/Combobox.vue'
import Label from '../components/ui/Label.vue'
import Card from '../components/ui/Card.vue'
import Badge from '../components/ui/Badge.vue'
import Dialog from '../components/ui/Dialog.vue'
import DiffPane from '../components/DiffPane.vue'

const traces = shallowRef<Recording[]>([])
const callId = ref('')
const apiKey = ref('')
const temperature = ref<number | null>(null)
const maxTokens = ref<number | null>(null)
const confirmOpen = ref(false)
const running = ref(false)
const error = ref<string | null>(null)

interface ResultView {
  old: Recording
  newText: string
  oldText: string
  newUsage?: unknown
  newLatencyMs?: number
}
const result = shallowRef<ResultView | null>(null)

async function load() {
  traces.value = await api.traces({ limit: 200 })
  if (!callId.value && traces.value.length) callId.value = traces.value[0].call_id
}
onMounted(load)

function recLatency(r: Recording): number {
  return r.exchanges?.reduce((s, e) => s + (e.timing?.latency_ms ?? 0), 0) ?? 0
}

function oldText(r: Recording): string {
  const input = r.input as unknown as { prompt?: Array<{ role?: string; content?: Array<{ type?: string; text?: string }> }> }
  const parts: string[] = []
  for (const m of input.prompt ?? []) {
    const text = (m.content ?? [])
      .filter((c) => c.type === 'text' && c.text)
      .map((c) => c.text)
      .join(' ')
    if (text) parts.push(`[${m.role}] ${text}`)
  }
  return parts.join('\n')
}

async function replay() {
  if (!callId.value) return
  error.value = null
  running.value = true
  try {
    const old = traces.value.find((t) => t.call_id === callId.value)
    if (!old) throw new Error('recording not found')
    const res = await api.replay({
      call_id: callId.value,
      api_key: apiKey.value || null,
      overrides: {
        temperature: temperature.value,
        max_output_tokens: maxTokens.value,
      },
    })
    result.value = {
      old,
      oldText: oldText(old),
      newText: res.text,
      newUsage: res.usage,
      newLatencyMs: typeof res.meta === 'object' && res.meta ? undefined : undefined,
    }
    confirmOpen.value = false
  } catch (e) {
    error.value = String(e)
  } finally {
    running.value = false
  }
}
</script>

<template>
  <div class="flex h-full flex-col">
    <div class="flex flex-wrap items-end gap-2 border-b px-4 py-2">
      <span class="pb-2 text-sm font-semibold">Replay</span>
      <div class="w-72">
        <Label>选择录制（请求回放，重发真实 API）</Label>
        <Combobox v-model="callId" :options="traces.map((t) => t.call_id)" class="mt-1" placeholder="call_id…" />
      </div>
      <div class="w-44">
        <Label>API key（env，explicit 源需要）</Label>
        <Input v-model="apiKey" class="mt-1 font-mono" placeholder="env:VAR" />
      </div>
      <div class="w-32">
        <Label>Temperature</Label>
        <Input v-model="temperature" type="number" step="0.1" class="mt-1 font-mono" placeholder="沿用录制" />
      </div>
      <div class="w-36">
        <Label>Max tokens</Label>
        <Input v-model="maxTokens" type="number" class="mt-1 font-mono" placeholder="沿用录制" />
      </div>
      <Button :disabled="running || !callId" @click="confirmOpen = true">
        {{ running ? '重放中…' : '重放 →' }}
      </Button>
    </div>

    <div class="flex-1 overflow-auto p-4">
      <div v-if="error" class="mb-3"><Badge variant="destructive">{{ error }}</Badge></div>

      <template v-if="result">
        <div class="mb-3 flex items-center gap-3 text-xs text-muted-foreground">
          <span class="font-mono">{{ result.old.call_id.slice(0, 24) }}…</span>
          <span>旧延迟 {{ recLatency(result.old) }}ms</span>
          <span>·</span>
          <span>新延迟 <span v-if="result.newLatencyMs != null">{{ result.newLatencyMs }}ms</span><span v-else>—</span></span>
          <span class="ml-auto">输入相同，输出差异如下</span>
        </div>
        <DiffPane :old-text="result.oldText" :new-text="result.newText" old-title="原输出" new-title="新输出" />

        <div class="mt-4">
          <div class="mb-2 text-xs font-semibold text-muted-foreground">新调用 usage</div>
          <Card class="p-2"><pre class="font-mono text-xs whitespace-pre-wrap">{{ JSON.stringify(result.newUsage, null, 2) }}</pre></Card>
        </div>
      </template>

      <div v-else class="flex h-full items-center justify-center text-sm text-muted-foreground">
        选择一条录制，重发真实 API 对比新旧输出（消耗真实费用）。
      </div>
    </div>

    <Dialog v-model:open="confirmOpen" title="确认重放">
      <p class="text-sm text-muted-foreground">
        请求回放会调用 <span class="font-mono">{{ callId.slice(0, 24) }}…</span> 对应的 provider 并消耗
        <b class="text-foreground">真实 API 费用与 token</b>。确认继续？
      </p>
      <div class="mt-4 flex justify-end gap-2">
        <Button variant="ghost" @click="confirmOpen = false">取消</Button>
        <Button variant="destructive" @click="replay">确认重放</Button>
      </div>
    </Dialog>
  </div>
</template>
