import { defineStore } from 'pinia'
import { ref } from 'vue'
import { api } from '../api/client'

export interface ConsoleTool {
  name: string
  description?: string | null
  parameters: unknown
}

export const useAppStore = defineStore('app', () => {
  const providers = ref<string[]>([])
  const suggestedModels = ref<Record<string, string[]>>({})
  const tools = ref<ConsoleTool[]>([])
  const loaded = ref(false)
  const mockMode = ref(false)
  const backendError = ref<string | null>(null)

  async function load() {
    if (loaded.value) return
    try {
      const [p, t] = await Promise.all([api.providers(), api.tools()])
      providers.value = p.providers
      suggestedModels.value = p.suggested_models
      tools.value = t.tools
      loaded.value = true
      backendError.value = null
    } catch (e) {
      backendError.value = String(e)
      console.error('failed to load providers/tools', e)
    }
  }

  function newSessionId(): string {
    return `sess-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`
  }

  return { providers, suggestedModels, tools, loaded, mockMode, backendError, load, newSessionId }
})
