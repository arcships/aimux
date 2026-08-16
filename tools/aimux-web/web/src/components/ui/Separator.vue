<script setup lang="ts">
import { cn } from '../../lib/utils'
withDefaults(defineProps<{ class?: string }>(), {})
</script>

<template>
  <div :class="cn('h-px w-full bg-border', $props.class)" />
</template>
