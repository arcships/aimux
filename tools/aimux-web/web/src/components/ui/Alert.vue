<script setup lang="ts">
import { computed } from 'vue'
import { cn } from '../../lib/utils'

const props = withDefaults(defineProps<{ variant?: 'default' | 'destructive' | 'warning' | 'success' }>(), {
  variant: 'default',
})
const cls = computed(() =>
  cn('relative w-full rounded-lg border px-4 py-3 text-sm', {
    default: 'border-border bg-muted text-foreground',
    destructive: 'border-destructive/40 bg-destructive/10 text-destructive',
    warning: 'border-warning/40 bg-warning/10 text-warning',
    success: 'border-success/40 bg-success/10 text-success',
  }[props.variant]),
)
</script>

<template>
  <div :class="cls"><slot /></div>
</template>
