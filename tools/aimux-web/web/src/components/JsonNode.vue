<script setup lang="ts">
import { computed, ref } from 'vue'

const props = defineProps<{ value: unknown; path: string; label?: string }>()

const open = ref(true)
const isObj = computed(
  () => typeof props.value === 'object' && props.value !== null && !Array.isArray(props.value),
)
const isArr = computed(() => Array.isArray(props.value))
const collapsible = computed(() => isObj.value || isArr.value)
const entries = computed(() => {
  if (isArr.value) return (props.value as unknown[]).map((v, i) => [String(i), v] as const)
  if (isObj.value) return Object.entries(props.value as Record<string, unknown>)
  return []
})

function renderScalar(v: unknown): { text: string; cls: string } {
  if (v === null) return { text: 'null', cls: 'text-muted-foreground italic' }
  if (typeof v === 'string') {
    if (v === '[REDACTED]')
      return { text: '"<REDACTED>"', cls: 'text-warning bg-warning/10 rounded px-0.5' }
    return { text: JSON.stringify(v), cls: 'text-success' }
  }
  if (typeof v === 'number') return { text: String(v), cls: 'text-sky-400' }
  if (typeof v === 'boolean') return { text: String(v), cls: 'text-purple-400' }
  return { text: String(v), cls: 'text-foreground' }
}
</script>

<template>
  <div>
    <div class="flex items-start gap-1 hover:bg-accent/30 rounded px-0.5">
      <button
        v-if="collapsible"
        class="w-4 text-muted-foreground hover:text-foreground cursor-pointer select-none"
        @click="open = !open"
      >
        {{ open ? '▾' : '▸' }}
      </button>
      <span v-else class="w-4" />
      <span v-if="label !== undefined" class="text-foreground">{{ label }}:</span>
      <template v-if="collapsible">
        <span class="text-muted-foreground">
          {{ isArr ? '[' : '{' }}
          <span v-if="!open" class="text-muted-foreground">
            …{{ isArr ? (value as unknown[]).length : Object.keys(value as object).length }} items
          </span>
        </span>
        <template v-if="open">
          <div class="w-full">
            <JsonNode v-for="(child, i) in entries" :key="i" :value="child[1]" :path="`${path}.${child[0]}`" :label="child[0]" />
          </div>
        </template>
        <span class="text-muted-foreground">{{ isArr ? ']' : '}' }}</span>
      </template>
      <template v-else>
        <span :class="renderScalar(value).cls">{{ renderScalar(value).text }}</span>
      </template>
    </div>
  </div>
</template>
