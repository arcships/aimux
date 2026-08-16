<script setup lang="ts">
import { cn } from '../../lib/utils'
withDefaults(defineProps<{ class?: string }>(), {})
</script>

<template>
  <div :class="cn('animate-pulse rounded-md bg-secondary/70', $props.class)" />
</template>
