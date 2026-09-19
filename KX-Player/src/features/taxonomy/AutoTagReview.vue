<script setup lang="ts">
/** 自动打标审核弹窗（P0-13）：建议先预览、用户确认后才写库。支持一键全选。 */
import { ref, computed, onMounted, watch } from 'vue'
import { useUiStore } from '@/stores/ui'
import { useLibraryStore } from '@/stores/library'
import { useTaxonomyStore } from '@/stores/taxonomy'
import type { TagSuggestion } from '@/contracts/api'

interface Item {
  trackId: string
  trackName: string
  suggestions: TagSuggestion[]
  accepted: boolean[]
}

const ui = useUiStore()
const library = useLibraryStore()
const taxonomy = useTaxonomyStore()

const items = ref<Item[]>([])
const loading = ref(false)

const acceptedCount = computed(() =>
  items.value.reduce((n, it) => n + it.accepted.filter(Boolean).length, 0),
)

async function load(): Promise<void> {
  const trackIds = ui.autoTagReview?.trackIds ?? []
  if (!trackIds.length) return
  loading.value = true
  try {
    const res = await taxonomy.suggestTags(trackIds)
    items.value = res
      .map(({ trackId, suggestions }) => {
        const t = library.trackIndex.get(trackId)
        return {
          trackId,
          trackName: t ? t.name : trackId,
          suggestions,
          accepted: suggestions.map(() => true), // 一键全选为默认
        }
      })
      .filter((it) => it.suggestions.length > 0)
  } finally {
    loading.value = false
  }
}

function toggleAll(on: boolean): void {
  for (const it of items.value) it.accepted = it.suggestions.map(() => on)
}

async function apply(): Promise<void> {
  const accepted: { trackId: string; tagName: string }[] = []
  for (const it of items.value) {
    it.suggestions.forEach((s, i) => {
      if (it.accepted[i]) accepted.push({ trackId: it.trackId, tagName: s.name })
    })
  }
  await taxonomy.applySuggested(accepted)
  close()
}

function close(): void {
  ui.autoTagReview = null
}

function toTagMode(): void {
  ui.view = 'smart'
}

onMounted(load)
watch(() => ui.autoTagReview, (v) => { if (v) void load() })
</script>

<template>
  <div v-if="ui.autoTagReview" class="atr-overlay" @click.self="close">
    <div class="atr-modal" role="dialog" aria-label="自动打标审核">
      <div class="atr-head">
        <h3>自动打标建议</h3>
        <span class="atr-sub">关键词规则命中 {{ acceptedCount }} 条 · 确认后才会写入</span>
        <button class="atr-close" title="关闭" @click="close">✕</button>
      </div>

      <div v-if="loading" class="atr-loading">分析中…</div>
      <div v-else-if="!items.length" class="atr-loading">
        没有新建议（文件名/路径未命中 ASMR 关键词，或已打过对应标签）
      </div>

      <div v-else class="atr-list">
        <div v-for="it in items" :key="it.trackId" class="atr-item">
          <div class="atr-name" :title="it.trackName">{{ it.trackName }}</div>
          <div class="atr-sugs">
            <button
              v-for="(s, i) in it.suggestions"
              :key="s.name"
              class="atr-sug"
              :class="{ on: it.accepted[i] }"
              @click="it.accepted[i] = !it.accepted[i]"
            >
              #{{ s.name }}
            </button>
          </div>
        </div>
      </div>

      <div class="atr-foot">
        <button class="atr-btn" @click="toggleAll(true)">全选</button>
        <button class="atr-btn" @click="toggleAll(false)">全不选</button>
        <span class="atr-space" />
        <button class="atr-btn" @click="toTagMode">仅关闭</button>
        <button class="atr-btn atr-primary" :disabled="!acceptedCount" @click="apply">
          应用（{{ acceptedCount }}）
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.atr-overlay {
  position: fixed;
  inset: 0;
  z-index: 100;
  background: var(--modal-overlay);
  display: flex;
  align-items: center;
  justify-content: center;
}
.atr-modal {
  width: min(560px, calc(100vw - 80px));
  max-height: 70vh;
  display: flex;
  flex-direction: column;
  background: var(--modal-bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  box-shadow: 0 18px 60px rgba(0, 0, 0, 0.45);
  overflow: hidden;
}
.atr-head {
  display: flex;
  align-items: baseline;
  gap: 10px;
  padding: 14px 18px 10px;
}
.atr-head h3 {
  font-size: 14px;
}
.atr-sub {
  font-size: 11px;
  color: var(--text-muted);
  flex: 1;
}
.atr-close {
  color: var(--text-muted);
  font-size: 13px;
  padding: 2px 6px;
  border-radius: 4px;
}
.atr-close:hover { color: var(--text); background: var(--bg-hover); }
.atr-loading {
  padding: 24px 18px;
  font-size: 12px;
  color: var(--text-muted);
}
.atr-list {
  flex: 1;
  overflow-y: auto;
  padding: 4px 18px 8px;
}
.atr-item {
  padding: 8px 0;
  border-bottom: 1px solid var(--border);
}
.atr-item:last-child { border-bottom: none; }
.atr-name {
  font-size: 12px;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.atr-sugs {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 6px;
}
.atr-sug {
  font-size: 11px;
  padding: 2px 10px;
  border-radius: 999px;
  border: 1px solid var(--border-strong);
  color: var(--text-sub);
}
.atr-sug.on {
  color: rgb(var(--accent-rgb));
  border-color: rgb(var(--accent-rgb) / 0.6);
  background: var(--bg-selected);
}
.atr-foot {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 18px 14px;
}
.atr-space { flex: 1; }
.atr-btn {
  font-size: 12px;
  padding: 5px 12px;
  border-radius: var(--radius-sm);
  background: var(--bg-input);
  color: var(--text-sub);
}
.atr-btn:hover { color: var(--text); background: var(--bg-hover); }
.atr-primary {
  background: rgb(var(--accent-rgb) / 0.9);
  color: #fff;
}
.atr-primary:hover { background: rgb(var(--accent-rgb)); color: #fff; }
.atr-primary:disabled { opacity: 0.45; }
</style>
