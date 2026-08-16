<script setup lang="ts">
import JsonNode from './JsonNode.vue'

defineProps<{ data: unknown }>()
</script>

<template>
  <div class="font-mono text-xs leading-relaxed break-all">
    <JsonNode :value="data" path="$" />
  </div>
</template>
