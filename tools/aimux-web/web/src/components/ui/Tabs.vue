<script setup lang="ts">
import { cn } from '../../lib/utils'

withDefaults(
  defineProps<{ modelValue?: string; tabs: Array<{ value: string; label: string }> }>(),
  {},
)
const emit = defineEmits<{ 'update:modelValue': [v: string] }>()
</script>

<template>
  <div class="inline-flex h-9 items-center justify-center rounded-lg bg-muted p-1 text-muted-foreground">
    <button
      v-for="t in tabs"
      :key="t.value"
      type="button"
      :class="cn(
        'inline-flex items-center justify-center whitespace-nowrap rounded-md px-3 py-1 text-sm font-medium transition-all cursor-pointer',
        modelValue === t.value ? 'bg-background text-foreground shadow' : 'hover:text-foreground',
      )"
      @click="emit('update:modelValue', t.value)"
    >
      {{ t.label }}
    </button>
  </div>
</template>
