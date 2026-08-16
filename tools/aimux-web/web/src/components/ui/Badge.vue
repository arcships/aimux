<script setup lang="ts">
import { computed } from 'vue'
import { cn } from '../../lib/utils'

const props = withDefaults(
  defineProps<{ variant?: 'default' | 'secondary' | 'success' | 'warning' | 'destructive' | 'outline' }>(),
  { variant: 'default' },
)

const cls = computed(() =>
  cn(
    'inline-flex items-center rounded-md border px-2.5 py-0.5 text-xs font-semibold transition-colors',
    {
      default: 'border-transparent bg-primary text-primary-foreground',
      secondary: 'border-transparent bg-secondary text-secondary-foreground',
      success: 'border-transparent bg-success/15 text-success',
      warning: 'border-transparent bg-warning/15 text-warning',
      destructive: 'border-transparent bg-destructive/15 text-destructive',
      outline: 'text-foreground',
    }[props.variant],
  ),
)
</script>

<template>
  <span :class="cls"><slot /></span>
</template>
