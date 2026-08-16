<script setup lang="ts">
import { computed } from 'vue'
import { cn } from '../../lib/utils'

const props = withDefaults(
  defineProps<{
    variant?: 'default' | 'secondary' | 'outline' | 'ghost' | 'destructive' | 'success'
    size?: 'default' | 'sm' | 'lg' | 'icon'
    disabled?: boolean
  }>(),
  { variant: 'default', size: 'default', disabled: false },
)

const emit = defineEmits<{ click: [e: MouseEvent] }>()

const cls = computed(() =>
  cn(
    'inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium transition-colors',
    'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 cursor-pointer select-none',
    {
      default: 'bg-primary text-primary-foreground hover:bg-primary/90',
      secondary: 'bg-secondary text-secondary-foreground hover:bg-secondary/80',
      outline: 'border border-input bg-transparent hover:bg-accent hover:text-accent-foreground',
      ghost: 'hover:bg-accent hover:text-accent-foreground',
      destructive: 'bg-destructive text-destructive-foreground hover:bg-destructive/90',
      success: 'bg-success text-black hover:opacity-90',
    }[props.variant],
    {
      default: 'h-9 px-4 py-2',
      sm: 'h-8 rounded-md px-3 text-xs',
      lg: 'h-10 rounded-md px-8',
      icon: 'h-9 w-9',
    }[props.size],
  ),
)
</script>

<template>
  <button :class="cls" :disabled="disabled" @click="emit('click', $event)">
    <slot />
  </button>
</template>
