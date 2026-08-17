<script setup lang="ts">
import { onMounted } from 'vue'
import { RouterLink, RouterView } from 'vue-router'
import { useAppStore } from './stores/app'

const store = useAppStore()
onMounted(() => store.load())

const nav = [
  { path: '/', label: 'Playground', icon: '▣' },
  { path: '/agent', label: 'Agent', icon: '◈' },
  { path: '/traces', label: 'Traces', icon: '▤' },
  { path: '/sessions', label: 'Sessions', icon: '≡' },
  { path: '/replay', label: 'Replay', icon: '↻' },
  { path: '/cache-probe', label: 'Cache', icon: '◉' },
  { path: '/settings', label: 'Settings', icon: '⚙' },
]
</script>

<template>
  <div class="flex h-screen overflow-hidden">
    <!-- left nav -->
    <aside class="flex w-44 shrink-0 flex-col border-r bg-card">
      <div class="flex items-center gap-2 border-b px-3 py-3">
        <span class="text-lg font-bold tracking-tight">aimux</span>
        <span class="text-xs text-muted-foreground">web console</span>
      </div>
      <nav class="flex-1 space-y-1 p-2">
        <RouterLink
          v-for="item in nav"
          :key="item.path"
          :to="item.path"
          class="flex items-center gap-2 rounded-md px-2 py-1.5 text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
          :class="{ 'bg-accent text-accent-foreground': $route.path === item.path }"
        >
          <span class="w-4 text-center">{{ item.icon }}</span>
          <span>{{ item.label }}</span>
        </RouterLink>
      </nav>
      <div class="border-t p-3 text-xs text-muted-foreground">
        <div class="flex items-center gap-1">
          <span
            class="inline-block h-2 w-2 rounded-full"
            :class="store.backendError ? 'bg-destructive' : store.loaded ? 'bg-success' : 'bg-warning'"
          />
          {{ store.backendError ? 'API 不可用' : store.mockMode ? 'mock 模式' : 'local' }}
        </div>
        <div v-if="store.backendError" class="mt-1 break-all">{{ store.backendError }}</div>
      </div>
    </aside>

    <!-- main -->
    <main class="min-w-0 flex-1 overflow-hidden">
      <RouterView />
    </main>
  </div>
</template>
