<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { useAppStore } from '../stores/app'
import { api } from '../api/client'
import type { StoredKey } from '../api/client'
import Button from '../components/ui/Button.vue'
import Input from '../components/ui/Input.vue'
import Combobox from '../components/ui/Combobox.vue'
import Label from '../components/ui/Label.vue'
import Card from '../components/ui/Card.vue'
import Badge from '../components/ui/Badge.vue'
import Switch from '../components/ui/Switch.vue'
import Alert from '../components/ui/Alert.vue'
import Separator from '../components/ui/Separator.vue'

const store = useAppStore()

const provider = ref('openai')
const key = ref('')
const remember = ref(false)
const saved = ref<StoredKey[]>([])
const plaintextEntry = ref(true)
const loaded = ref(false)
const saving = ref(false)
const message = ref<string | null>(null)
const error = ref<string | null>(null)

async function load() {
  error.value = null
  try {
    const res = await api.settingsKeys()
    saved.value = res.keys
    plaintextEntry.value = res.plaintext_entry
    loaded.value = true
  } catch (e) {
    error.value = String(e)
  }
}
onMounted(() => {
  store.load()
  load()
})

async function save() {
  if (!provider.value.trim() || !key.value || saving.value) return
  error.value = null
  message.value = null
  saving.value = true
  try {
    const res = await api.putSettingsKey({
      provider: provider.value.trim(),
      key: key.value,
      remember: remember.value,
    })
    message.value = res.remembered
      ? `已保存 ${res.provider} 的 key（内存 + 磁盘）`
      : `已保存 ${res.provider} 的 key（仅本次运行，重启失效）`
    key.value = ''
    await load()
  } catch (e) {
    error.value = String(e)
  } finally {
    saving.value = false
  }
}

async function remove(p: string) {
  error.value = null
  message.value = null
  try {
    const res = await api.deleteSettingsKey(p)
    message.value = res.removed ? `已删除 ${p} 的 key` : `${p} 没有已保存的 key`
    await load()
  } catch (e) {
    error.value = String(e)
  }
}
</script>

<template>
  <div class="h-full overflow-auto p-6">
    <div class="mx-auto max-w-3xl space-y-6">
      <div class="flex items-center gap-2">
        <span class="text-sm font-semibold">Settings</span>
        <Badge variant="secondary" class="font-mono text-[10px]">API keys</Badge>
      </div>

      <Alert v-if="!plaintextEntry" variant="warning">
        服务绑定在非回环地址，网页内 key 管理已禁用。请在启动环境设置 <span class="font-mono">env:</span> 引用
        （如 <span class="font-mono">api_key="env:OPENAI_API_KEY"</span>）或 provider 注册的默认环境变量。
      </Alert>

      <Card class="p-4">
        <div class="space-y-4">
          <div>
            <Label>Provider</Label>
            <Combobox
              v-model="provider"
              :options="store.providers"
              class="mt-1"
              placeholder="搜索 provider…"
            />
          </div>
          <div>
            <Label>API key（明文仅保存在服务端）</Label>
            <Input
              v-model="key"
              type="password"
              autocomplete="off"
              class="mt-1 font-mono"
              placeholder="sk-…"
              :disabled="!plaintextEntry"
            />
            <div class="mt-1 text-[11px] text-muted-foreground">
              保存后所有页面（Playground / Agent / Replay / Cache）调用该 provider 时自动使用；GET
              列表只显示掩码，永不回传明文。
            </div>
          </div>
          <div class="flex items-center justify-between">
            <div>
              <Label>记住（写入磁盘）</Label>
              <div class="text-[11px] text-muted-foreground">
                写入配置目录 keys.json（权限 0600），重启后自动加载；不勾选仅保存在内存。
              </div>
            </div>
            <Switch v-model="remember" :disabled="!plaintextEntry" />
          </div>
          <div class="flex gap-2">
            <Button :disabled="!plaintextEntry || !key || saving" @click="save">
              {{ saving ? '保存中…' : '保存' }}
            </Button>
            <Button
              variant="outline"
              :disabled="!plaintextEntry || !provider"
              @click="remove(provider.trim())"
            >
              删除该 provider 的 key
            </Button>
          </div>
          <div v-if="message" class="text-xs text-success">{{ message }}</div>
          <div v-if="error"><Badge variant="destructive">{{ error }}</Badge></div>
        </div>
      </Card>

      <Separator />

      <div>
        <div class="mb-2 text-xs font-semibold text-muted-foreground">已保存的 key</div>
        <Card class="p-0">
          <table v-if="saved.length" class="w-full text-sm">
            <thead class="text-left text-xs text-muted-foreground">
              <tr>
                <th class="px-4 py-2">Provider</th>
                <th class="px-4 py-2">Key（掩码）</th>
                <th class="px-4 py-2">持久化</th>
                <th class="px-4 py-2" />
              </tr>
            </thead>
            <tbody>
              <tr v-for="k in saved" :key="k.provider" class="border-t">
                <td class="px-4 py-2 font-mono text-xs">{{ k.provider }}</td>
                <td class="px-4 py-2 font-mono text-xs text-muted-foreground">{{ k.hint ?? '—' }}</td>
                <td class="px-4 py-2 text-xs">
                  <Badge :variant="k.remembered ? 'default' : 'secondary'">
                    {{ k.remembered ? '磁盘' : '内存' }}
                  </Badge>
                </td>
                <td class="px-4 py-2 text-right">
                  <Button
                    variant="ghost"
                    size="sm"
                    :disabled="!plaintextEntry"
                    @click="remove(k.provider)"
                  >
                    删除
                  </Button>
                </td>
              </tr>
            </tbody>
          </table>
          <div v-else-if="loaded" class="px-4 py-6 text-center text-sm text-muted-foreground">
            还没有保存任何 key。
          </div>
        </Card>
        <div class="mt-2 text-[11px] text-muted-foreground">
          调用优先级：请求内显式 env: 引用 &gt; 这里保存的 key &gt; provider 注册的默认环境变量。
        </div>
      </div>
    </div>
  </div>
</template>
