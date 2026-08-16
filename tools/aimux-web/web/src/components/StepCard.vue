<script setup lang="ts">
import type { AgentStepView } from '../agent/engine'
import StreamMessage from './StreamMessage.vue'
import Badge from './ui/Badge.vue'

defineProps<{ step: AgentStepView }>()
const emit = defineEmits<{ trace: [] }>()
</script>

<template>
  <div class="mb-3">
    <div class="mb-1 flex items-center gap-2 text-[11px] text-muted-foreground">
      <span>step{{ step.step }}</span>
      <Badge :variant="step.status === 'done' ? 'success' : step.status === 'error' ? 'destructive' : step.status === 'stopped' ? 'secondary' : 'warning'">
        {{ step.status }}
      </Badge>
      <span v-if="step.latencyMs != null">{{ step.latencyMs }}ms</span>
      <button
        v-if="step.meta?.call_id"
        class="underline decoration-dotted hover:text-foreground cursor-pointer"
        @click="emit('trace')"
      >
        trace →
      </button>
    </div>

    <div v-if="step.text" class="mb-2 rounded-md border bg-card p-3">
      <StreamMessage :content="step.text" :final="step.status !== 'running'" />
    </div>

    <div v-for="tc in step.toolCalls" :key="tc.id" class="mb-2 rounded-md border bg-muted/40 p-2 font-mono text-xs">
      <div class="flex items-center gap-2">
        <span class="font-semibold">⟐ {{ tc.name }}</span>
        <span v-if="tc.executing" class="animate-pulse text-warning">执行中…</span>
        <span v-if="tc.is_error" class="text-destructive">ERROR</span>
      </div>
      <div class="mt-1 break-all text-muted-foreground">in: {{ JSON.stringify(tc.input) }}</div>
      <div v-if="tc.result !== undefined" class="mt-1 break-all">out: {{ JSON.stringify(tc.result) }}</div>
    </div>

    <div v-if="step.error" class="mb-2 text-xs text-destructive">{{ step.error }}</div>
  </div>
</template>
