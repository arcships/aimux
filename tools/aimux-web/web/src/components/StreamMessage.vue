<script setup lang="ts">
import MarkdownRender from 'markstream-vue'

withDefaults(defineProps<{ content: string; final?: boolean }>(), { final: true })
</script>

<template>
  <MarkdownRender mode="chat" :content="content" :final="final" class="aimux-md" />
</template>

<style>
.aimux-md {
  color: var(--color-foreground);
  font-size: 0.875rem;
  line-height: 1.7;
}
</style>
