<script setup lang="ts">
import { computed, ref } from 'vue'
import { cn } from '../../lib/utils'
import Input from './Input.vue'

const props = withDefaults(
  defineProps<{
    modelValue?: string
    options: string[]
    placeholder?: string
    emptyText?: string
    class?: string
  }>(),
  { placeholder: '选择…', emptyText: '无匹配项' },
)

const emit = defineEmits<{ 'update:modelValue': [v: string] }>()
const open = ref(false)
const query = ref('')

const filtered = computed(() => {
  const q = query.value.trim().toLowerCase()
  if (!q) return props.options
  return props.options.filter((o) => o.toLowerCase().includes(q))
})

function pick(v: string) {
  emit('update:modelValue', v)
  open.value = false
  query.value = ''
}
</script>

<template>
  <div class="relative">
    <button
      type="button"
      :class="cn('h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-left text-sm shadow-sm hover:bg-accent focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring cursor-pointer truncate', modelValue ? 'text-foreground' : 'text-muted-foreground', $props.class)"
      @click="open = !open"
    >
      {{ modelValue || placeholder }}
    </button>
    <div v-if="open" class="absolute z-50 mt-1 w-full rounded-md border bg-popover text-popover-foreground shadow-lg">
      <div class="p-1">
        <Input v-model="query" placeholder="过滤…" class="h-8" />
      </div>
      <div class="max-h-56 overflow-auto p-1">
        <button
          v-for="o in filtered"
          :key="o"
          type="button"
          class="flex w-full items-center rounded-sm px-2 py-1.5 text-left text-sm hover:bg-accent hover:text-accent-foreground cursor-pointer"
          @click="pick(o)"
        >
          <span class="truncate">{{ o }}</span>
        </button>
        <div v-if="!filtered.length" class="px-2 py-3 text-center text-xs text-muted-foreground">
          {{ emptyText }}
        </div>
      </div>
    </div>
  </div>
</template>
