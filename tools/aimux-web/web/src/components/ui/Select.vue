<script setup lang="ts">
import { cn } from '../../lib/utils'

const props = withDefaults(defineProps<{ modelValue?: string; options: Array<{ label: string; value: string }>; class?: string }>(), {})
const emit = defineEmits<{ 'update:modelValue': [v: string] }>()
</script>

<template>
  <select
    :value="modelValue"
    class="h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 cursor-pointer"
    @change="emit('update:modelValue', ($event.target as HTMLSelectElement).value)"
  >
    <option v-for="o in options" :key="o.value" :value="o.value" class="bg-background">
      {{ o.label }}
    </option>
  </select>
</template>
