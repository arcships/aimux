<script setup lang="ts">
import { onMounted, ref, shallowRef } from 'vue'
import { useRouter } from 'vue-router'
import { api } from '../api/client'
import type { SessionView } from '../types/SessionView'
import type { Recording } from '../types/Recording'
import Badge from '../components/ui/Badge.vue'
import Skeleton from '../components/ui/Skeleton.vue'

const router = useRouter()
const sessions = ref<SessionView[]>([])
const loading = ref(false)
const selected = ref<string | null>(null)
const recordings = shallowRef<Recording[]>([])

async function load() {
  loading.value = true
  try {
    sessions.value = await api.sessions()
  } finally {
    loading.value = false
  }
}

async function open(id: string) {
  selected.value = id
  const d = await api.sessionDetail(id)
  recordings.value = d.recordings
}

onMounted(load)

function recLatency(r: Recording): number {
  return r.exchanges?.reduce((s, e) => s + (e.timing?.latency_ms ?? 0), 0) ?? 0
}

function verdictBadge(r: Recording): string {
  // verdicts live in TraceRecord; for recordings show outcome only here
  return String(r.outcome?.status ?? '').toLowerCase()
}
</script>

<template>
  <div class="flex h-full min-w-0">
    <div class="flex w-80 shrink-0 flex-col border-r">
      <div class="flex items-center gap-2 border-b px-4 py-2">
        <span class="text-sm font-semibold">Sessions</span>
        <div class="flex-1" />
        <button class="text-xs text-muted-foreground hover:text-foreground cursor-pointer" @click="load">刷新</button>
      </div>
      <div class="flex-1 overflow-auto">
        <template v-if="loading">
          <div v-for="i in 6" :key="i" class="p-3"><Skeleton class="h-10" /></div>
        </template>
        <template v-else>
          <button
            v-for="s in sessions"
            :key="s.session_id"
            class="block w-full border-b px-4 py-3 text-left hover:bg-accent/40 cursor-pointer"
            :class="{ 'bg-accent/60': selected === s.session_id }"
            @click="open(s.session_id)"
          >
            <div class="flex items-center gap-2">
              <span class="truncate font-mono text-xs">{{ s.session_id }}</span>
              <Badge variant="secondary" class="text-[10px]">{{ s.source }}</Badge>
              <span class="ml-auto text-xs text-muted-foreground">{{ s.calls.length }} calls</span>
            </div>
            <div class="mt-1 text-[11px] text-muted-foreground">
              最后：{{ s.calls[s.calls.length - 1]?.recorded_at?.replace('T', ' ').slice(0, 19) ?? '—' }}
            </div>
          </button>
          <div v-if="!sessions.length" class="p-6 text-center text-xs text-muted-foreground">
            暂无会话 — Agent/Playground 带 session_id 的调用会归组到这里
          </div>
        </template>
      </div>
    </div>

    <div class="min-w-0 flex-1 overflow-auto p-4">
      <div v-if="!selected" class="flex h-full items-center justify-center text-sm text-muted-foreground">
        选择一个会话查看调用链
      </div>
      <template v-else>
        <div class="mb-3 flex items-center gap-2">
          <span class="font-mono text-sm">{{ selected }}</span>
          <Badge variant="secondary">{{ recordings.length }} recordings</Badge>
        </div>
        <div class="space-y-2">
          <div
            v-for="r in [...recordings].sort((a, b) => (a.step ?? 0) - (b.step ?? 0))"
            :key="r.call_id"
            class="flex cursor-pointer items-center gap-3 rounded-md border px-3 py-2 hover:bg-accent/40"
            @click="router.push({ path: '/traces', query: { call: r.call_id } })"
          >
            <span class="w-12 font-mono text-xs text-muted-foreground">step{{ r.step }}</span>
            <span class="w-36 truncate font-mono text-xs">{{ r.provider?.provider }}/{{ r.provider?.model_id }}</span>
            <span class="text-xs text-muted-foreground">{{ recLatency(r) }}ms</span>
            <span class="ml-auto"><Badge :variant="verdictBadge(r) === 'error' ? 'destructive' : verdictBadge(r) === 'success' ? 'success' : 'secondary'">{{ r.outcome?.status }}</Badge></span>
          </div>
          <div v-if="!recordings.length" class="text-xs text-muted-foreground">（该会话的录制已不在环形缓冲区）</div>
        </div>
      </template>
    </div>
  </div>
</template>
