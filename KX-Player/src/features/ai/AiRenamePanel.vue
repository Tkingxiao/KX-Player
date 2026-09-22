<script setup lang="ts">
/**
 * 改名主干（AI-6/7/10，`04 §7`）的 **dry-run 面板**：列目录 → 14 级判定 → 本地规范化 → 找冲突。
 *
 * 判定、规范化、冲突检测全在 Rust（`src-tauri/src/rename/`），这里只把账摊开给人看。
 * 本批**没有任何写盘路径**：`rename_preview` 只读目录，应用与回滚要到 S2 才存在，
 * 所以这一屏无论点什么都不动得了一个字节 —— 标题里把这件事写明，别让人猜。
 */
import { computed, ref } from 'vue'
import { useUiStore } from '@/stores/ui'
import { api } from '@/bridge/ipc'
import { errText } from '@/contracts/result'
import type { RenamePreview, RenamePreviewItem, RenameReason, RenameVerdict } from '@/contracts/api'

const props = defineProps<{ targetDir?: string }>()

const ui = useUiStore()
const roots = ref<string[]>([])
const busy = ref(false)
const result = ref<RenamePreview | null>(null)

const VERDICT_LABEL: Record<RenameVerdict, string> = {
  skip: '跳过',
  normalize_only: '只需规范化',
  needs_repair: '要修',
  needs_translation: '要翻',
  compliant: '已合规',
}

const REASON_LABEL: Record<RenameReason, string> = {
  locked: '已锁定',
  rejected: '已拒绝过',
  illegal_char: '形近字符',
  whitespace: '空格',
  wave: '波浪号',
  half_full: '半全角',
  overly_long: '超长',
  prefix_mixed: '序号族混用',
  duplicate_title: '重复文件夹名',
  mt_smell: '机翻痕迹',
  kana: '含假名',
  traditional: '含繁体',
  roman: '罗马字',
}

const CONFLICT_LABEL = {
  same_target: '这批里撞名',
  target_exists: '目标已存在',
  path_too_long: '全路径超长',
} as const

const base = (p: string): string => p.split(/[\\/]/).pop() ?? p

const counts = computed(() => result.value?.counts)
const items = computed(() => result.value?.items ?? [])
/** 还要花 token 的那几条：规则修不掉的，S2 会送进模型 */
const aiCount = computed(() => items.value.filter((i) => i.needsAi).length)
const conflictList = computed(() => items.value.filter((i) => i.conflict))

const VERDICT_ORDER: RenameVerdict[] = ['skip', 'normalize_only', 'needs_repair', 'needs_translation', 'compliant']
const shown = computed(() => {
  const c = counts.value
  if (!c) return []
  return VERDICT_ORDER.map((v) => ({ v, n: c[verdictKey[v]] }))
})
const verdictKey: Record<RenameVerdict, 'skip' | 'normalizeOnly' | 'needsRepair' | 'needsTranslation' | 'compliant'> = {
  skip: 'skip',
  normalize_only: 'normalizeOnly',
  needs_repair: 'needsRepair',
  needs_translation: 'needsTranslation',
  compliant: 'compliant',
}

async function addRoot(): Promise<void> {
  const picked = await api.openFolder()
  if (!picked?.length) return
  const fresh = picked.filter((p) => !roots.value.includes(p))
  if (!fresh.length) {
    ui.toast('这些目录已经在列表里了')
    return
  }
  roots.value = [...roots.value, ...fresh]
}

function useTargetDir(): void {
  const d = props.targetDir
  if (!d) return
  if (roots.value.includes(d)) return
  roots.value = [...roots.value, d]
}

function removeRoot(p: string): void {
  roots.value = roots.value.filter((r) => r !== p)
  result.value = null
}

async function run(): Promise<void> {
  if (!roots.value.length || busy.value) return
  busy.value = true
  try {
    result.value = await api.renamePreview(roots.value)
  } catch (e) {
    result.value = null
    ui.toast(errText(e), 'error')
  } finally {
    busy.value = false
  }
}

function rowCls(i: RenamePreviewItem): string {
  return `v-${i.verdict}${i.conflict ? ' bad' : ''}`
}
</script>

<template>
  <div class="rn">
    <div class="rn-row">
      <h3 class="section-title rn-title">改名预览（规则规范化）· 只预览，不改一个文件</h3>
      <div class="rn-actions">
        <button class="btn-ghost" @click="addRoot">添加目录…</button>
        <button v-if="targetDir" class="btn-ghost" @click="useTargetDir">用当前目标文件夹</button>
        <button class="btn-primary" :disabled="!roots.length || busy" @click="run">
          {{ busy ? '计算中…' : '算一遍' }}
        </button>
      </div>
    </div>

    <div v-if="roots.length" class="rn-roots">
      <span v-for="r in roots" :key="r" class="rn-chip" :title="r">
        {{ base(r) }}
        <button class="rn-x" title="移出列表" @click="removeRoot(r)">×</button>
      </span>
    </div>

    <p class="rn-hint">
      规则先做能做的（全角半角、波浪号、连续空格、结尾句点、时间表达、超长截断），剩下要翻译或修句子的
      才交给模型。<b>这一屏不落库、不重命名</b>：应用与回滚是下一批的事。
    </p>

    <template v-if="result">
      <div class="rn-counts">
        <span>共 {{ counts?.scanned }} 条</span>
        <span v-for="s in shown" :key="s.v" :class="`v-${s.v}`">{{ VERDICT_LABEL[s.v] }} {{ s.n }}</span>
        <span>有改动 {{ counts?.changed }}</span>
        <span :class="{ bad: (counts?.conflicts ?? 0) > 0 }">冲突 {{ counts?.conflicts }}</span>
        <span>要送模型 {{ aiCount }}</span>
      </div>
      <p v-if="result.truncated" class="rn-warn">
        条目只列出前 300 条（计数仍是全量），把范围拆小一点再看。
      </p>
      <p v-for="u in result.unreadable" :key="`u-${u}`" class="rn-warn">列不出来：{{ u }}</p>

      <ul v-if="items.length" class="rn-items">
        <li v-for="i in items" :key="i.path" :class="rowCls(i)">
          <span class="rn-kind">{{ i.kind === 'folder' ? '目录' : '文件' }}</span>
          <span class="rn-verdict">{{ VERDICT_LABEL[i.verdict] }}</span>
          <span class="rn-from" :title="i.path">{{ i.name }}</span>
          <span class="rn-arrow">→</span>
          <span class="rn-to" :title="i.target">{{ i.target }}</span>
          <span v-if="i.reasons.length" class="rn-reasons">
            {{ i.reasons.map((r) => REASON_LABEL[r]).join('、') }}
          </span>
          <span v-if="i.needsAi" class="rn-ai">要模型</span>
          <span v-if="i.conflict" class="rn-conflict">{{ CONFLICT_LABEL[i.conflict] }}</span>
        </li>
      </ul>
      <div v-else class="rn-empty">这一层里没有需要动的条目 —— 全部已合规。</div>

      <p v-if="conflictList.length" class="rn-warn">
        {{ conflictList.length }} 条撞名或超长，将来即使有应用按钮也不会自动改它们。
      </p>
    </template>
    <div v-else-if="roots.length" class="rn-empty">点「算一遍」看这批目录会怎么改。</div>
    <div v-else class="rn-empty">先添加要检查的目录。</div>
  </div>
</template>

<style scoped>
.rn {
  display: flex;
  flex-direction: column;
  gap: 7px;
}
.rn-row { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; justify-content: space-between; }
.rn-title { margin: 0; }
.rn-actions { display: flex; flex-wrap: wrap; gap: 8px; }
.rn-hint { font-size: 11.5px; line-height: 1.65; color: var(--text-muted); }

.rn-roots { display: flex; flex-wrap: wrap; gap: 6px; }
.rn-chip {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 2px 4px 2px 8px;
  border: 1px solid var(--border);
  border-radius: 999px;
  font-size: 11.5px;
  color: var(--text-sub);
  max-width: 260px;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}
.rn-x { border: 0; background: none; color: var(--text-muted); cursor: pointer; font-size: 13px; line-height: 1; padding: 0 4px; }
.rn-x:hover { color: var(--danger); }

.rn-counts { display: flex; flex-wrap: wrap; gap: 6px 14px; font-size: 11.5px; color: var(--text-sub); }
.rn-counts .bad { color: var(--danger); }
.rn-warn { font-size: 11.5px; color: var(--warning); word-break: break-all; }
.rn-empty { font-size: 11.5px; color: var(--text-muted); }

.rn-items {
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 3px;
  max-height: 260px;
  overflow-y: auto;
  font-size: 11.5px;
}
.rn-items li { display: flex; align-items: baseline; gap: 8px; color: var(--text-sub); }
.rn-kind,
.rn-verdict { flex: 0 0 auto; color: var(--text-muted); }
.rn-verdict { min-width: 5.5em; }
.rn-from { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text-muted); }
.rn-arrow { color: var(--text-muted); }
.rn-to { color: var(--text); word-break: break-all; }
.rn-reasons { color: var(--text-muted); }
.rn-ai { color: rgb(var(--accent-rgb)); }
.rn-conflict,
.rn-items li.bad .rn-to { color: var(--danger); }
.rn-items li.v-skip .rn-verdict { color: var(--text-muted); }
.rn-items li.v-compliant .rn-verdict { color: var(--success); }
.rn-items li.v-needs_repair .rn-verdict,
.rn-items li.v-needs_translation .rn-verdict { color: var(--warning); }
</style>
