<script setup lang="ts">
import { computed, onMounted, ref, shallowRef, watch } from 'vue'
import { useRoute } from 'vue-router'
import { useAppStore } from '../stores/app'
import { api } from '../api/client'
import type { Recording } from '../types/Recording'
import Button from '../components/ui/Button.vue'
import Input from '../components/ui/Input.vue'
import Textarea from '../components/ui/Textarea.vue'
import Select from '../components/ui/Select.vue'
import Tabs from '../components/ui/Tabs.vue'
import Badge from '../components/ui/Badge.vue'
import Card from '../components/ui/Card.vue'
import Dialog from '../components/ui/Dialog.vue'
import Skeleton from '../components/ui/Skeleton.vue'
import JsonViewer from '../components/JsonViewer.vue'
import Waterfall, { type WaterfallItem } from '../components/Waterfall.vue'
import type { WireMeta } from '../types/WireMeta'

const store = useAppStore()
const route = useRoute()

const traces = shallowRef<Recording[]>([])
const loading = ref(false)
const filterProvider = ref('')
const filterSession = ref('')
const filterStatus = ref('')
const searchCallId = ref('')
const selected = shallowRef<Recording | null>(null)
const detailTab = ref('input')
const importOpen = ref(false)
const importText = ref('')
const importMsg = ref('')

async function load() {
  loading.value = true
  try {
    traces.value = await api.traces({
      provider: filterProvider.value || undefined,
      session: filterSession.value || undefined,
      status: filterStatus.value || undefined,
      limit: 300,
    })
    if (searchCallId.value) {
      traces.value = traces.value.filter((t) => t.call_id.includes(searchCallId.value))
    }
  } finally {
    loading.value = false
  }
}

async function selectCallId(callId: string) {
  try {
    selected.value = await api.trace(callId)
  } catch {
    selected.value = null
  }
}

onMounted(() => {
  load()
  const call = route.query.call as string | undefined
  if (call) selectCallId(call)
})

watch([filterProvider, filterSession, filterStatus], () => load())

function statusVariant(s: string): 'default' | 'success' | 'destructive' | 'warning' | 'secondary' {
  if (s === 'success') return 'success'
  if (s === 'error') return 'destructive'
  if (s === 'incomplete' || s === 'cancelled') return 'warning'
  return 'secondary'
}

function recLatency(r: Recording): number {
  return r.exchanges?.reduce((s, e) => s + (e.timing?.latency_ms ?? 0), 0) ?? 0
}

function recTokens(r: Recording): number | null {
  const u = r.outcome?.usage as { input_tokens?: { total?: number }; output_tokens?: { total?: number } } | null | undefined
  if (!u) return null
  const input = u.input_tokens?.total ?? 0
  const output = u.output_tokens?.total ?? 0
  return input + output
}

function timeLabel(r: Recording): string {
  return r.recorded_at.replace('T', ' ').replace('Z', '').slice(0, 19)
}

function providerLabel(r: Recording): string {
  return `${r.provider?.provider ?? '?'}/${r.provider?.model_id ?? '?'}`
}

const sessionItems = computed<WaterfallItem[]>(() => {
  const sid = selected.value?.session_id
  if (!sid) return []
  return traces.value
    .filter((t) => t.session_id === sid)
    .map((t) => ({
      step: t.step ?? 0,
      label: `${t.provider?.model_id ?? 'model'}`,
      ms: recLatency(t),
      ttfb: t.exchanges?.[0]?.timing?.ttfb_ms ?? null,
      status: String(t.outcome?.status ?? '').toLowerCase(),
    }))
})

async function exportJsonl() {
  const text = await api.exportJsonl()
  const blob = new Blob([text], { type: 'application/x-ndjson' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = `aimux-recordings-${Date.now()}.jsonl`
  a.click()
  URL.revokeObjectURL(url)
}

async function doImport() {
  importMsg.value = ''
  try {
    const res = await api.importJsonl(importText.value)
    importMsg.value = `已导入 ${res.imported} 条`
    importText.value = ''
    await load()
  } catch (e) {
    importMsg.value = String(e)
  }
}

function traceMetaOf(r: Recording): WireMeta {
  return {
    call_id: r.call_id,
    session_id: r.session_id ?? null,
    step: r.step ?? null,
    outcome: String(r.outcome?.status ?? '').toLowerCase(),
  }
}
</script>

<template>
  <div class="flex h-full min-w-0">
    <div class="flex min-w-0 flex-1 flex-col">
      <div class="flex flex-wrap items-center gap-2 border-b px-4 py-2">
        <span class="text-sm font-semibold">Traces</span>
        <Select v-model="filterProvider" class="w-36" :options="[{ label: '全部 provider', value: '' }, ...store.providers.map((p) => ({ label: p, value: p }))]" />
        <Input v-model="filterSession" class="w-40" placeholder="session id 过滤" />
        <Select v-model="filterStatus" class="w-32" :options="[{ label: '全部状态', value: '' }, { label: 'success', value: 'success' }, { label: 'error', value: 'error' }, { label: 'incomplete', value: 'incomplete' }, { label: 'cancelled', value: 'cancelled' }]" />
        <Input v-model="searchCallId" class="w-44" placeholder="call_id 搜索" @keydown.enter="load" />
        <div class="flex-1" />
        <Button variant="outline" size="sm" @click="importOpen = true">导入</Button>
        <Button variant="outline" size="sm" @click="exportJsonl">导出 jsonl</Button>
        <Button variant="outline" size="sm" @click="load">刷新</Button>
      </div>

      <div class="flex-1 overflow-auto">
        <table class="w-full text-sm">
          <thead class="sticky top-0 bg-background text-left text-xs text-muted-foreground">
            <tr>
              <th class="px-3 py-2">时间</th>
              <th class="px-3 py-2">provider / model</th>
              <th class="px-3 py-2">session / step</th>
              <th class="px-3 py-2 text-right">延迟</th>
              <th class="px-3 py-2 text-right">tokens</th>
              <th class="px-3 py-2">状态</th>
            </tr>
          </thead>
          <tbody>
            <template v-if="loading">
              <tr v-for="i in 8" :key="i"><td colspan="6" class="px-3 py-2"><Skeleton class="h-6" /></td></tr>
            </template>
            <template v-else>
              <tr
                v-for="r in traces"
                :key="r.call_id"
                class="cursor-pointer border-t transition-colors hover:bg-accent/40"
                :class="{ 'bg-accent/60': selected?.call_id === r.call_id }"
                @click="selected = r"
              >
                <td class="px-3 py-2 font-mono text-xs text-muted-foreground">{{ timeLabel(r) }}</td>
                <td class="px-3 py-2 font-mono text-xs">{{ providerLabel(r) }}</td>
                <td class="px-3 py-2 font-mono text-xs text-muted-foreground">
                  {{ r.session_id ? `${r.session_id.slice(0, 16)}…` : '—' }}<span v-if="r.step != null"> / {{ r.step }}</span>
                </td>
                <td class="px-3 py-2 text-right font-mono text-xs">{{ recLatency(r) }}ms</td>
                <td class="px-3 py-2 text-right font-mono text-xs">{{ recTokens(r) ?? '—' }}</td>
                <td class="px-3 py-2"><Badge :variant="statusVariant(String(r.outcome?.status ?? '').toLowerCase())">{{ r.outcome?.status }}</Badge></td>
              </tr>
              <tr v-if="!traces.length">
                <td colspan="6" class="px-3 py-10 text-center text-muted-foreground">
                  暂无录制 — 去 Playground 或 Agent 跑一次调用
                </td>
              </tr>
            </template>
          </tbody>
        </table>
      </div>
    </div>

    <!-- detail -->
    <aside v-if="selected" class="flex w-[30rem] shrink-0 flex-col border-l">
      <div class="flex items-center gap-2 border-b px-3 py-2">
        <span class="truncate font-mono text-xs">{{ selected.call_id }}</span>
        <div class="flex-1" />
        <Button variant="ghost" size="sm" @click="selected = null">✕</Button>
      </div>

      <div class="border-b p-3">
        <div class="mb-2 text-xs font-semibold text-muted-foreground">会话瀑布</div>
        <Waterfall :items="sessionItems.length ? sessionItems : [{ step: selected.step ?? 0, label: providerLabel(selected), ms: recLatency(selected), ttfb: selected.exchanges?.[0]?.timing?.ttfb_ms ?? null, status: String(selected.outcome?.status ?? '').toLowerCase() }]" />
      </div>

      <div class="flex-1 overflow-auto p-3">
        <Tabs v-model="detailTab" class="mb-3" :tabs="[{ value: 'input', label: '输入' }, { value: 'provider', label: 'Provider' }, { value: 'http', label: 'HTTP' }]" />

        <div v-if="detailTab === 'input'">
          <div class="mb-2 text-xs font-semibold text-muted-foreground">Prompt（消息）</div>
          <Card class="p-2 mb-3"><JsonViewer :data="selected.input?.prompt" /></Card>
          <div class="mb-2 text-xs font-semibold text-muted-foreground">Options</div>
          <Card class="p-2"><JsonViewer :data="selected.input?.options" /></Card>
        </div>

        <div v-if="detailTab === 'provider'">
          <Card class="p-2"><JsonViewer :data="selected.provider" /></Card>
        </div>

        <div v-if="detailTab === 'http'">
          <div v-if="!selected.exchanges?.length" class="text-xs text-muted-foreground">（无 HTTP 交换）</div>
          <div v-for="(ex, i) in selected.exchanges" :key="i" class="mb-3">
            <div class="mb-1 flex items-center gap-2 text-xs">
              <Badge variant="secondary">attempt {{ ex.attempt }}</Badge>
              <span class="text-muted-foreground">{{ ex.request?.method }} {{ ex.request?.url }}</span>
              <span v-if="ex.response" class="text-muted-foreground">→ {{ ex.response.status }}</span>
              <span class="ml-auto text-muted-foreground">{{ ex.timing?.latency_ms ?? '—' }}ms</span>
            </div>
            <Card class="p-2 mb-1">
              <div class="mb-1 text-[11px] text-muted-foreground">请求 body</div>
              <JsonViewer :data="ex.request?.body" />
            </Card>
            <Card class="p-2">
              <div class="mb-1 text-[11px] text-muted-foreground">响应 body{{ ex.response?.stream_chunks != null ? `（${ex.response.stream_chunks} chunks）` : '' }}</div>
              <JsonViewer :data="ex.response?.body" />
            </Card>
          </div>
        </div>
      </div>
    </aside>
  </div>
</template>
