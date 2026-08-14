<script setup lang="ts">
import { cn } from '../../lib/utils'

withDefaults(
  defineProps<{
    modelValue?: string | number | null
    type?: string
    placeholder?: string
    disabled?: boolean
    step?: number | string
    min?: number | string
    class?: string
  }>(),
  { type: 'text' },
)

const emit = defineEmits<{ 'update:modelValue': [v: string | number] }>()

function onInput(e: Event) {
  const el = e.target as HTMLInputElement
  emit('update:modelValue', el.type === 'number' ? Number(el.value) : el.value)
}
</script>

<template>
  <input
    :type="type"
    :placeholder="placeholder"
    :disabled="disabled"
    :step="step"
    :min="min"
    :value="modelValue ?? ''"
    class="h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
    @input="onInput"
  />
</template>
