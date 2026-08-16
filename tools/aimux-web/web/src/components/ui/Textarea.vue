<script setup lang="ts">
import { cn } from '../../lib/utils'

withDefaults(defineProps<{ modelValue?: string; placeholder?: string; rows?: number; disabled?: boolean; class?: string }>(), {
  rows: 3,
})

const emit = defineEmits<{ 'update:modelValue': [v: string] }>()
</script>

<template>
  <textarea
    :placeholder="placeholder"
    :rows="rows"
    :disabled="disabled"
    :value="modelValue"
    class="w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-sm placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 font-mono"
    @input="emit('update:modelValue', ($event.target as HTMLTextAreaElement).value)"
  />
</template>
