<script setup lang="ts">
import { computed } from 'vue'
import { diffLines } from '../lib/diff'

const props = defineProps<{
  oldText: string
  newText: string
  oldTitle?: string
  newTitle?: string
}>()

const ops = computed(() => diffLines(props.oldText, props.newText))
const stats = computed(() => {
  let add = 0
  let del = 0
  for (const op of ops.value) {
    if (op.type === 'add') add++
    else if (op.type === 'del') del++
  }
  return { add, del, same: ops.value.length - add - del }
})
</script>

<template>
  <div class="rounded-md border">
    <div class="flex items-center justify-between border-b px-3 py-2 text-xs text-muted-foreground">
      <span>{{ oldTitle ?? '原' }} → {{ newTitle ?? '新' }}</span>
      <span>
        <span class="text-success">+{{ stats.add }}</span>
        <span class="mx-1">·</span>
        <span class="text-destructive">−{{ stats.del }}</span>
        <span class="mx-1">·</span>
        <span>{{ stats.same }} same</span>
      </span>
    </div>
    <div class="max-h-[24rem] overflow-auto font-mono text-xs leading-relaxed p-2">
      <div
        v-for="(op, i) in ops"
        :key="i"
        class="whitespace-pre-wrap break-all px-2 py-px rounded-sm"
        :class="{
          'bg-success/10 text-success': op.type === 'add',
          'bg-destructive/10 text-destructive line-through': op.type === 'del',
          'text-muted-foreground': op.type === 'same',
        }"
      >
        <span class="mr-2 select-none opacity-60">{{ op.type === 'add' ? '+' : op.type === 'del' ? '−' : ' ' }}</span>{{ op.text || '⏎' }}
      </div>
    </div>
  </div>
</template>
