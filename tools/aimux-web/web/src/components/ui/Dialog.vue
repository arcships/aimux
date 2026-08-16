<script setup lang="ts">
import { cn } from '../../lib/utils'

withDefaults(defineProps<{ open?: boolean; title?: string; class?: string }>(), {})
const emit = defineEmits<{ close: [] }>()
</script>

<template>
  <Teleport to="body">
    <div v-if="open" class="fixed inset-0 z-50 flex items-center justify-center">
      <div class="absolute inset-0 bg-black/60" @click="emit('close')" />
      <div :class="cn('relative z-10 w-full max-w-lg rounded-lg border bg-card p-5 text-card-foreground shadow-xl', $props.class)">
        <h2 v-if="title" class="mb-3 text-base font-semibold">{{ title }}</h2>
        <slot />
      </div>
    </div>
  </Teleport>
</template>
