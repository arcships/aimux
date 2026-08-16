<script setup lang="ts">
import { cn } from '../../lib/utils'

withDefaults(defineProps<{ modelValue?: boolean; disabled?: boolean }>(), {})
const emit = defineEmits<{ 'update:modelValue': [v: boolean] }>()
</script>

<template>
  <button
    type="button"
    role="switch"
    :aria-checked="modelValue"
    :disabled="disabled"
    :class="cn(
      'inline-flex h-5 w-9 shrink-0 items-center rounded-full border-2 border-transparent transition-colors cursor-pointer',
      'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50',
      modelValue ? 'bg-primary' : 'bg-input',
    )"
    @click="emit('update:modelValue', !modelValue)"
  >
    <span
      :class="cn('pointer-events-none block h-4 w-4 rounded-full bg-background shadow-lg ring-0 transition-transform', modelValue ? 'translate-x-4' : 'translate-x-0')"
    />
  </button>
</template>
