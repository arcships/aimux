<script setup lang="ts">
import { ref } from 'vue'
import { useAppStore } from '../stores/app'
import { api } from '../api/client'
import Button from '../components/ui/Button.vue'
import Input from '../components/ui/Input.vue'
import Textarea from '../components/ui/Textarea.vue'
import Combobox from '../components/ui/Combobox.vue'
import Label from '../components/ui/Label.vue'
import Card from '../components/ui/Card.vue'
import Badge from '../components/ui/Badge.vue'
import Skeleton from '../components/ui/Skeleton.vue'

const store = useAppStore()
const provider = ref('deepseek')
const model = ref('deepseek-chat')
const apiKey = ref('')
const baseUrl = ref('')
const maxRequests = ref(4)
const prompt = ref('')
const dryRun = ref(false)
const running = ref(false)
const error = ref<string | null>(null)

interface RoundRow {
  round: number
  cache_read_tokens?: number
  input_total_tokens?: number | null
  output_tokens?: number | null
  elapsed_ms?: number
  text_preview?: string
  error?: string
}
interface ProbeResult {
  provider: string
  model: string
  rounds: RoundRow[]
  stats?: unknown
  records?: unknown[]
  dry_run?: boolean
}
const result = ref<ProbeResult | null>(null)

async function run() {
  error.value = null
  running.value = true
  try {
    result.value = (await api.cacheProbe({
      provider: provider.value,
      model: model.value,
      api_key: apiKey.value || null,
      base_url: baseUrl.value || null,
      max_requests: maxRequests.value,
      prompt: prompt.value || null,
      dry_run: dryRun.value,
    })) as ProbeResult
  } catch (e) {
    error.value = String(e)
  } finally {
    running.value = false
  }
}

function hitRate(): number | null {
  const withClaim = result.value?.rounds.filter((r) => r.cache_read_tokens != null)
  if (!withClaim || !withClaim.length) return null
  const read = withClaim.reduce((s, r) => s + (r.cache_read_tokens ?? 0), 0)
  const total = withClaim.reduce((s, r) => s + (r.input_total_tokens ?? 0), 0)
  return total > 0 ? Math.round((read / total) * 100) : null
}
</script>

<template>
  <div class="flex h-full flex-col">
    <div class="flex flex-wrap items-end gap-2 border-b px-4 py-2">
      <span class="pb-2 text-sm font-semibold">Cache Probe</span>
      <div class="w-40">
        <Label>Provider</Label>
        <Combobox v-model="provider" :options="store.providers" class="mt-1" placeholder="provider…" />
      </div>
      <div class="w-44">
        <Label>Model</Label>
        <Input v-model="model" class="mt-1 font-mono" placeholder="model id" />
      </div>
      <div class="w-44">
        <Label>API key（env）</Label>
        <Input v-model="apiKey" class="mt-1 font-mono" placeholder="env:VAR 或留空" />
      </div>
      <div class="w-40">
        <Label>Base URL</Label>
        <Input v-model="baseUrl" class="mt-1 font-mono" placeholder="可选" />
      </div>
      <div class="w-28">
        <Label>Requests</Label>
        <Input v-model="maxRequests" type="number" min="1" class="mt-1 font-mono" />
      </div>
      <label class="flex items-center gap-2 pb-2 text-sm cursor-pointer">
        <input v-model="dryRun" type="checkbox" class="accent-foreground" />
        dry-run
      </label>
      <Button :disabled="running" variant="default" @click="run">{{ running ? '探测中…' : '运行 →' }}</Button>
    </div>

    <div class="flex-1 overflow-auto p-4">
      <div v-if="error" class="mb-3"><Badge variant="destructive">{{ error }}</Badge></div>

      <div v-if="result?.dry_run" class="rounded-md border p-4 text-sm text-muted-foreground">
        dry-run：将发送 {{ result.rounds }} 个请求（provider {{ result.provider }} / {{ result.model }}，session "cache-probe"），未调用真实 API。
      </div>

      <template v-else-if="result">
        <div class="mb-4 grid grid-cols-3 gap-3">
          <Card class="p-3">
            <div class="text-xs text-muted-foreground">命中率（cache_read / input）</div>
            <div class="mt-1 text-2xl font-bold">{{ hitRate() != null ? `${hitRate()}%` : '—' }}</div>
          </Card>
          <Card class="p-3">
            <div class="text-xs text-muted-foreground">轮次</div>
            <div class="mt-1 text-2xl font-bold">{{ result.rounds.length }}</div>
          </Card>
          <Card class="p-3">
            <div class="text-xs text-muted-foreground">失败轮次</div>
            <div class="mt-1 text-2xl font-bold">{{ result.rounds.filter((r) => r.error).length }}</div>
          </Card>
        </div>

        <table class="w-full text-sm">
          <thead class="text-left text-xs text-muted-foreground">
            <tr>
              <th class="px-3 py-2">#</th>
              <th class="px-3 py-2 text-right">cache_read</th>
              <th class="px-3 py-2 text-right">input</th>
              <th class="px-3 py-2 text-right">output</th>
              <th class="px-3 py-2 text-right">延迟</th>
              <th class="px-3 py-2">预览</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="r in result.rounds" :key="r.round" class="border-t">
              <td class="px-3 py-2 font-mono text-xs">{{ r.round }}</td>
              <td v-if="r.error" colspan="5" class="px-3 py-2 font-mono text-xs text-destructive">{{ r.error }}</td>
              <template v-else>
                <td class="px-3 py-2 text-right font-mono text-xs" :class="r.cache_read_tokens ? 'text-success' : 'text-muted-foreground'">{{ r.cache_read_tokens }}</td>
                <td class="px-3 py-2 text-right font-mono text-xs">{{ r.input_total_tokens ?? '—' }}</td>
                <td class="px-3 py-2 text-right font-mono text-xs">{{ r.output_tokens ?? '—' }}</td>
                <td class="px-3 py-2 text-right font-mono text-xs">{{ r.elapsed_ms }}ms</td>
                <td class="px-3 py-2 font-mono text-xs text-muted-foreground">{{ r.text_preview }}</td>
              </template>
            </tr>
          </tbody>
        </table>

        <div class="mt-4">
          <div class="mb-2 text-xs font-semibold text-muted-foreground">汇总统计（TraceStats）</div>
          <Card class="p-2"><pre class="font-mono text-xs whitespace-pre-wrap">{{ JSON.stringify(result.stats, null, 2) }}</pre></Card>
        </div>
      </template>

      <div v-else-if="running" class="space-y-2">
        <Skeleton v-for="i in 4" :key="i" class="h-10" />
      </div>

      <div v-else class="flex h-full items-center justify-center text-sm text-muted-foreground">
        在线探测 provider 的 prefix 缓存能力（每轮追加对话内容验证前缀命中，消耗真实费用）。
      </div>
    </div>
  </div>
</template>
