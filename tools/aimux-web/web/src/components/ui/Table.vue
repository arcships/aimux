<script setup lang="ts">
import { cn } from '../../lib/utils'
withDefaults(defineProps<{ class?: string }>(), {})
</script>

<template>
  <div :class="cn('w-full overflow-auto rounded-md border', $props.class)">
    <table class="w-full caption-bottom text-sm">
      <slot />
    </table>
  </div>
</template>
