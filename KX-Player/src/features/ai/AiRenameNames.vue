<script setup lang="ts">
/**
 * 改名主干里**问模型**的那一半（AI-9，`04 §7.6~7.7`）：清点片段 → 一段一次请求 → 过九道后置校验
 * → 核准的名字写进译名库。
 *
 * 这一格**一个字都不改盘**：拿到名字之后仍然要按上面面板的「应用改名」才会动文件，而 apply 查的
 * 是同一张表、同一份判据 —— 所以这里说「已核准」的那几条，就是下一次真会被改的那几条（不多也不少）。
 * 键是「提示词版本 + 片段」的内容键，因此同一批名字再生成一次一个 token 都不花。
 *
 * 没有确认弹窗：不改盘的操作不需要 danger 确认，而按错了也不过是问模型要了几个名字，
 * 不想要就在清单里点「不要这个译名」（`04 §7.7` 第 9 条）。
 */
import { computed, ref } from 'vue'
import { useUiStore } from '@/stores/ui'
import { useSettingsStore } from '@/stores/settings'
import { api } from '@/bridge/ipc'
import { errText } from '@/contracts/result'
import type { RenameSuggestReport } from '@/contracts/api'
import { apiKey, jobOpts } from './useAiAccess'

const props = defineProps<{ roots: string[]; aiCount: number; busy: boolean }>()
const emit = defineEmits<{ (e: 'refresh'): void; (e: 'busy', on: boolean): void }>()

const ui = useUiStore()
const settings = useSettingsStore()
const working = ref(false)
const rep = ref<RenameSuggestReport | null>(null)

/** 外接模型没填 Key 就别按：一次要发几十上百个请求，全数失败只是白等一场 */
const noKey = computed(() => settings.aiModelKind === 'remote' && !apiKey.value)
const blocked = computed(() => props.busy || !props.roots.length || props.aiCount === 0 || noKey.value)
const hint = computed(() => {
  if (!props.roots.length) return '先添加目录'
  if (noKey.value) return '外接模型还没填 API Key —— 到「API」那一格填好'
  if (props.aiCount === 0) return '这一批里没有需要模型的条目'
  return `向模型要 ${props.aiCount} 个名字，一次最多 200 个，中途没有取消按钮`
})

async function generate(): Promise<void> {
  if (working.value || blocked.value) return
  working.value = true
  emit('busy', true)
  try {
    const o = jobOpts()
    const r = await api.renameSuggestNames(props.roots, o.baseURL, o.apiKey, o.model, o.modelKind)
    rep.value = r
    ui.toast(
      `译名：新拿到 ${r.generated} 个${r.cached ? `，${r.cached} 个本来就有` : ''}` +
        `${r.review.length || r.failed.length ? ` · 没拿到 ${r.review.length + r.failed.length}` : ''}`,
      r.generated || r.cached ? 'success' : 'error',
    )
    // 名字进了库，预览上那些条目就该从「要模型」变成「可应用」
    emit('refresh')
  } catch (e) {
    ui.toast(errText(e), 'error')
  } finally {
    working.value = false
    emit('busy', false)
  }
}
</script>

<template>
  <div class="rn-names">
    <div class="rn-names-row">
      <p class="rn-names-hint">{{ hint }}。规则修不掉的才送出去 —— 送之前已经把序号前缀和扩展名剥掉了。</p>
      <button class="btn-ghost" :disabled="blocked || working" @click="generate">
        {{ working ? '生成中…' : `生成译名（${aiCount}）` }}
      </button>
    </div>

    <div v-if="rep" class="rn-names-rep">
      <span>模型 {{ rep.model }}</span>
      <span>提示词 {{ rep.promptVersion }}</span>
      <span>要 {{ rep.needed }} 个</span>
      <span class="good">命中缓存 {{ rep.cached }}</span>
      <span class="good">这次拿到 {{ rep.generated }}</span>
      <span v-if="rep.review.length" class="bad">过不了校验 {{ rep.review.length }}</span>
      <span v-if="rep.failed.length" class="bad">没问到 {{ rep.failed.length }}</span>
      <ul v-if="rep.review.length" class="rn-names-list">
        <li v-for="(r, k) in rep.review" :key="`rv-${k}`">要人看：{{ r }}</li>
      </ul>
      <ul v-if="rep.failed.length" class="rn-names-list">
        <li v-for="(r, k) in rep.failed" :key="`fl-${k}`">没问到：{{ r }}</li>
      </ul>
      <p class="rn-names-note">
        这些名字现在只是**账**：按上面「应用改名」才会动文件，而按下去之前你随时能在清单里拒掉某一条。
      </p>
    </div>
  </div>
</template>

<style scoped>
.rn-names { display: flex; flex-direction: column; gap: 5px; }
.rn-names-row { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; justify-content: space-between; }
.rn-names-hint { font-size: 11.5px; line-height: 1.6; color: var(--text-muted); margin: 0; }
.rn-names-rep {
  display: flex;
  flex-wrap: wrap;
  gap: 6px 14px;
  padding: 5px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  font-size: 11.5px;
  color: var(--text-sub);
}
.rn-names-rep .good { color: var(--success); }
.rn-names-rep .bad { color: var(--danger); }
.rn-names-list {
  flex: 1 0 100%;
  list-style: none;
  display: flex; flex-direction: column; gap: 2px;
  margin: 0;
  padding-left: 10px;
  font-size: 11.5px;
  color: var(--danger);
  word-break: break-all;
}
.rn-names-note { flex: 1 0 100%; font-size: 11.5px; color: var(--text-muted); margin: 0; }
</style>
