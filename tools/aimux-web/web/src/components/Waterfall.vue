<script setup lang="ts">
import { computed } from 'vue'

export interface WaterfallItem {
  step: number
  label: string
  ms: number
  ttfb?: number | null
  status: string
}

const props = defineProps<{ items: WaterfallItem[] }>()

const maxMs = computed(() => Math.max(1, ...props.items.map((i) => i.ms)))

function color(status: string): string {
  if (status === 'success') return '#22c55e'
  if (status === 'error') return '#ef4444'
  if (status === 'cancelled' || status === 'incomplete') return '#f59e0b'
  return '#71717a'
}
</script>

<template>
  <div class="space-y-2 font-mono text-xs">
    <div v-if="!items.length" class="text-muted-foreground">（无调用）</div>
    <div v-for="item in items" :key="item.step" class="flex items-center gap-2">
      <span class="w-14 shrink-0 text-muted-foreground">step{{ item.step }}</span>
      <span class="w-24 shrink-0 truncate" :title="item.label">{{ item.label }}</span>
      <div class="relative h-5 flex-1 overflow-hidden rounded-sm bg-muted">
        <div
          class="absolute inset-y-0 left-0 rounded-sm"
          :style="{
            width: `${Math.max(2, (item.ms / maxMs) * 100)}%`,
            background: color(item.status),
            opacity: 0.85,
          }"
        />
        <div
          v-if="item.ttfb"
          class="absolute inset-y-0 border-l border-background/70"
          :style="{ left: `${(item.ttfb / maxMs) * 100}%` }"
          :title="`TTFB ${item.ttfb}ms`"
        />
      </div>
      <span class="w-16 shrink-0 text-right text-muted-foreground">{{ item.ms }}ms</span>
    </div>
  </div>
</template>
