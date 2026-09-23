<!--
  模型选择器（AI-4/5）：一个「可输入 + 可下拉」的组合框。
  下拉是自己画的：原生 datalist 在长列表下会被截断，用户看不到后面那些模型。
  列表来自 `ai_list_models`，只在点「获取模型」时才请求 —— 不自动打接口，省一次无谓的联网。
  选中值直接写 `settings.aiModel`：三个工种共用同一个模型名，不在这里再抄一份状态。
-->
<script setup lang="ts">
import { computed, ref } from 'vue'
import { useUiStore } from '@/stores/ui'
import { useSettingsStore } from '@/stores/settings'
import { api } from '@/bridge/ipc'
import { errText } from '@/contracts/result'
import { apiKey } from './useAiAccess'

const ui = useUiStore()
const settings = useSettingsStore()

const modelList = ref<string[]>([])
const fetching = ref(false)
const error = ref('')
const open = ref(false)
const filter = ref('')

/** 已获取的模型按输入过滤（不区分大小写），用于下拉展示 */
const visibleModels = computed(() => {
  const q = filter.value.trim().toLowerCase()
  if (!q) return modelList.value
  return modelList.value.filter((m) => m.toLowerCase().includes(q))
})

function pick(m: string): void {
  settings.aiModel = m
  settings.scheduleSave()
  open.value = false
}

async function fetchModels(): Promise<void> {
  if (!settings.aiBaseURL.trim()) { error.value = '请先填写接口地址'; return }
  fetching.value = true
  error.value = ''
  try {
    const r = await api.aiListModels({ baseURL: settings.aiBaseURL, apiKey: apiKey.value })
    if (r.ok && r.models.length) {
      modelList.value = r.models
      if (!settings.aiModel || !r.models.includes(settings.aiModel)) {
        settings.aiModel = r.models[0]
        settings.scheduleSave()
      }
      ui.toast(`已获取 ${r.models.length} 个模型`, 'success')
      open.value = true
    } else {
      error.value = r.error || '未获取到模型'
    }
  } catch (e) {
    // 之前只有 try/finally：invoke 失败会被静默吞掉，按钮看起来「无效」
    error.value = errText(e)
  } finally {
    fetching.value = false
  }
}
</script>

<template>
  <label class="ai-field">
    <span>模型</span>
    <div class="ai-model-row">
      <div class="ai-model-combo">
        <input
          v-model="settings.aiModel"
          :placeholder="modelList.length ? '选择或输入模型' : '例如 gpt-4o-mini'"
          @change="settings.scheduleSave()"
          @focus="open = modelList.length > 0"
        />
        <button
          v-if="modelList.length"
          class="ai-model-toggle"
          title="展开模型列表"
          aria-label="展开模型列表"
          @click="open = !open"
        >
          <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.4"><polyline points="6,9 12,15 18,9" /></svg>
        </button>
      </div>
      <button
        class="btn-ghost ai-model-btn"
        :disabled="fetching || !settings.aiBaseURL.trim()"
        title="从接口获取可用模型列表"
        @click="fetchModels"
      >{{ fetching ? '获取中…' : '获取模型' }}</button>
    </div>

    <!-- 模型下拉：完整列出，可滚动、可过滤（替代原生 datalist 的截断显示） -->
    <Transition name="pop-scale">
      <div v-if="open && modelList.length" class="ai-model-menu">
        <div class="ai-model-menu-head">
          <input
            v-model="filter"
            class="ai-model-search"
            type="text"
            placeholder="过滤模型…"
            aria-label="过滤模型"
          />
          <span class="ai-model-count tnum">{{ visibleModels.length }}/{{ modelList.length }}</span>
        </div>
        <div class="ai-model-list">
          <button
            v-for="m in visibleModels"
            :key="m"
            class="ai-model-item"
            :class="{ active: m === settings.aiModel }"
            :title="m"
            @click="pick(m)"
          >
            <span class="ai-model-name">{{ m }}</span>
            <svg v-if="m === settings.aiModel" viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.6"><polyline points="20,6 9,17 4,12" /></svg>
          </button>
          <div v-if="!visibleModels.length" class="ai-model-empty">没有匹配的模型</div>
        </div>
      </div>
    </Transition>

    <span v-if="error" class="ai-field-error">{{ error }}</span>
  </label>
</template>

<style scoped>
/* 左列的一次性配置：绝对定位的下拉以这个 label 为 containing block */
.ai-field {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 4px;
  font-size: 11.5px;
  color: var(--text-muted);
  min-width: 0;
}
.ai-field input {
  padding: 7px 10px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
  background: var(--bg-input);
  font-size: 12px;
}
.ai-field input:focus { border-color: rgb(var(--accent-rgb)); }

.ai-model-row {
  display: flex;
  gap: 6px;
  align-items: center;
}
/* 输入框 + 展开箭头组合（占满剩余宽度） */
.ai-model-combo {
  position: relative;
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
}
.ai-model-combo input {
  flex: 1;
  min-width: 0;
  padding-right: 26px;
}
.ai-model-toggle {
  position: absolute;
  right: 4px;
  top: 50%;
  transform: translateY(-50%);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  border-radius: 4px;
  color: var(--text-muted);
}
.ai-model-toggle:hover { background: var(--bg-hover); color: var(--text); }
.ai-model-btn { flex-shrink: 0; padding: 6px 10px; font-size: 11px; }
.ai-field-error { font-size: 10.5px; color: var(--danger); }

/* ── 模型下拉列表：完整展示、可滚动、可过滤 ── */
.ai-model-menu {
  position: absolute;
  top: calc(100% + 6px);
  left: 0;
  right: 0;
  z-index: 60;
  border-radius: var(--radius);
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  box-shadow: var(--shadow-pop);
  overflow: hidden;
}
.ai-model-menu-head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px;
  border-bottom: 1px solid var(--border);
}
.ai-model-search {
  flex: 1;
  min-width: 0;
  padding: 5px 8px !important;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
  background: var(--bg-input);
  font-size: 12px;
}
.ai-model-count { font-size: 10.5px; color: var(--text-muted); flex-shrink: 0; }
/* 固定最大高度并滚动，保证长列表每一项都能看到 */
.ai-model-list {
  max-height: 260px;
  overflow-y: auto;
  padding: 5px;
}
.ai-model-item {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 7px 9px;
  border-radius: var(--radius-sm);
  font-size: 12px;
  color: var(--text-sub);
  text-align: left;
}
.ai-model-item:hover { background: var(--bg-hover); color: var(--text); }
.ai-model-item.active {
  color: rgb(var(--accent-rgb));
  background: var(--bg-selected);
  font-weight: 500;
}
.ai-model-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.ai-model-empty {
  padding: 14px 10px;
  text-align: center;
  font-size: 11.5px;
  color: var(--text-muted);
}
</style>
