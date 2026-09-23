<script setup lang="ts">
/**
 * 改名主干（AI-6/7/10，`04 §7`）的操作面板：列目录 → 14 级判定 → 本地规范化 → 找冲突 → **应用**。
 *
 * 判定、规范化、冲突检测、重命名、记账、回滚全在 Rust（`src-tauri/src/rename/`），这里只把账摊开
 * 给人看，再决定动不动手。「算一遍」与「生成译名」都只读盘（后者只写库），**只有「应用改名」会碰
 * 文件** —— 而它动的只有定稿的那些：本地规则改得动的，加上模型已经给过、又过了九道校验的。
 */
import { computed, ref } from 'vue'
import { useUiStore } from '@/stores/ui'
import { useLibraryStore } from '@/stores/library'
import { api } from '@/bridge/ipc'
import { errText } from '@/contracts/result'
import confirmHost from '@/services/confirmHost'
import type { RenamePreview, RenameReport } from '@/contracts/api'
import { base } from './renameLabels'
import AiRenameCounts from './AiRenameCounts.vue'
import AiRenameItems from './AiRenameItems.vue'
import AiRenameNames from './AiRenameNames.vue'
import AiRenameBatches from './AiRenameBatches.vue'

const props = defineProps<{ targetDir?: string }>()

const ui = useUiStore()
const library = useLibraryStore()
const roots = ref<string[]>([])
const busy = ref(false)
const result = ref<RenamePreview | null>(null)
/** 最近一次「应用」的汇总；「算一遍」不清它 —— 改完先看结果，再看新账 */
const report = ref<RenameReport | null>(null)
/** 传给批次表：+1 就重读一次 rename_batches */
const tick = ref(0)

/** 「生成译名」在跑：它不改盘，但那时应用会读到一本只写了一半的账，所以两个按钮互相挡 */
const aiBusy = ref(false)
const counts = computed(() => result.value?.counts)
const items = computed(() => result.value?.items ?? [])
/** 还要花 token 的那几条：规则修不掉的，点了「生成译名」才会去问模型 */
const aiCount = computed(() => items.value.filter((i) => i.needsAi).length)
const conflictList = computed(() => items.value.filter((i) => i.conflict))
/**
 * 应用真正会动的条数 —— 与后端 `queue()` 的筛选条件是同一条规则，别在这儿放宽：
 * 撞名的排除；要模型的那条只算**库里有核准译名**的（`suggested` 就是后端盖章的那一个）。
 */
const applicable = computed(
  () => items.value.filter((i) => !i.conflict && (i.needsAi ? !!i.suggested : i.changed)).length,
)
/** 清单被截断时上面只是个下限：后端按全量计划，如实标成「N+」 */
const more = computed(() => (result.value?.truncated ? '+' : ''))

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
  report.value = null
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

async function runApply(): Promise<void> {
  if (!result.value || busy.value || aiBusy.value || !applicable.value) return
  const ok = await confirmHost.open({
    title: `应用改名（${applicable.value}${more.value} 项）`,
    message:
      `把 ${applicable.value}${more.value} 个名字改掉：目录先改、子项跟着改，每改一项立刻记一笔账。` +
      '撞名的、以及还要问模型却又没有核准译名的，一条不动。改完下方「改名批次」里可以整批回滚。',
    confirmText: '改名',
    danger: true,
  })
  if (!ok) return
  busy.value = true
  try {
    const rep = await api.renameApply(roots.value)
    report.value = rep
    ui.toast(
      `已改 ${rep.applied}/${rep.planned} 项${rep.skipped ? `，跳过 ${rep.skipped}` : ''}` +
        `${rep.failed.length ? ` · 失败 ${rep.failed.length}` : ''}`,
      rep.failed.length ? 'error' : 'success',
    )
    // 名字变了，曲库里的路径也就变了：不重扫就是一堆「文件不存在」
    await library.rescan(true)
    tick.value += 1
    await run()
  } catch (e) {
    ui.toast(errText(e), 'error')
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <div class="rn">
    <div class="rn-row">
      <h3 class="section-title rn-title">改名（规则规范化）· 只有「应用改名」会碰盘，整批可回滚</h3>
      <div class="rn-actions">
        <button class="btn-ghost" @click="addRoot">添加目录…</button>
        <button v-if="targetDir" class="btn-ghost" @click="useTargetDir">用当前目标文件夹</button>
        <button class="btn-ghost" :disabled="!roots.length || busy" @click="run">
          {{ busy ? '计算中…' : '算一遍' }}
        </button>
        <button
          class="btn-primary"
          :disabled="!result || busy || aiBusy || !applicable"
          :title="!result ? '先算一遍' : applicable ? `重命名 ${applicable} 项，可整批回滚` : '这一批没有已经定稿的条目'"
          @click="runApply"
        >
          {{ busy ? '进行中…' : `应用改名${result ? `（${applicable}${more}）` : ''}` }}
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
      才交给模型 —— 两步都在下面，而<b>只有「应用改名」这一步会动文件</b>，改完整批可以退回。
    </p>

    <div v-if="report" class="rn-report">
      <span>批次 {{ report.batchId }}</span>
      <span class="good">已改 {{ report.applied }}/{{ report.planned }}</span>
      <span>跳过 {{ report.skipped }}</span>
      <span v-if="report.failed.length" class="bad">失败 {{ report.failed.length }}</span>
      <span>
        历史 {{ report.historyWritten }} 条{{ report.historyFile ? '' : '（这一层没有 rename_history.json，没有新建）' }}
      </span>
      <ul v-if="report.failed.length" class="rn-failed">
        <li v-for="(f, k) in report.failed" :key="`f-${k}`">{{ f }}</li>
      </ul>
    </div>

    <template v-if="result">
      <AiRenameCounts v-if="counts" :counts="counts" :ai-count="aiCount" :applicable="applicable" :more="more" />
      <AiRenameNames :roots="roots" :ai-count="aiCount" :busy="busy" @busy="aiBusy = $event" @refresh="run" />
      <p v-if="result.truncated" class="rn-warn">
        条目只列出前 300 条（计数仍是全量），把范围拆小一点再看。
      </p>
      <p v-for="u in result.unreadable" :key="`u-${u}`" class="rn-warn">列不出来：{{ u }}</p>

      <AiRenameItems v-if="items.length" :items="items" @refresh="run" />
      <div v-else class="rn-empty">这一层里没有需要动的条目 —— 全部已合规。</div>

      <p v-if="conflictList.length" class="rn-warn">
        {{ conflictList.length }} 条撞名或全路径超长，应用时一律跳过，改完也不会留账 —— 需要手动挪开再算一遍。
      </p>
    </template>
    <div v-else-if="roots.length" class="rn-empty">点「算一遍」看这批目录会怎么改。</div>
    <div v-else class="rn-empty">先添加要检查的目录。</div>

    <AiRenameBatches :tick="tick" />
  </div>
</template>

<style scoped>
.rn { display: flex; flex-direction: column; gap: 7px; }
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

.rn-report .bad { color: var(--danger); }
.rn-report .good { color: var(--success); }
.rn-warn { font-size: 11.5px; color: var(--warning); word-break: break-all; }
.rn-empty { font-size: 11.5px; color: var(--text-muted); }

.rn-report {
  display: flex;
  flex-wrap: wrap;
  gap: 6px 14px;
  padding: 5px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  font-size: 11.5px;
  color: var(--text-sub);
}
.rn-failed {
  flex: 1 0 100%;
  list-style: none;
  display: flex; flex-direction: column; gap: 2px;
  padding-left: 10px;
  font-size: 11.5px;
  color: var(--danger);
  word-break: break-all;
}
</style>
