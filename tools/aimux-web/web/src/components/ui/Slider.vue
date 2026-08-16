<script setup lang="ts">
withDefaults(defineProps<{ modelValue?: number; min?: number; max?: number; step?: number; disabled?: boolean }>(), {
  min: 0,
  max: 2,
  step: 0.1,
})
const emit = defineEmits<{ 'update:modelValue': [v: number] }>()
</script>

<template>
  <input
    type="range"
    :min="min"
    :max="max"
    :step="step"
    :disabled="disabled"
    :value="modelValue"
    class="h-2 w-full cursor-pointer appearance-none rounded-full bg-secondary accent-foreground"
    @input="emit('update:modelValue', Number(($event.target as HTMLInputElement).value))"
  />
</template>
