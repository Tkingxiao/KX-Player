<script setup lang="ts">
export type WorkflowStepState = 'pending' | 'active' | 'done' | 'failed'

export interface WorkflowStep {
  key: string
  label: string
  state: WorkflowStepState
  note?: string
}

defineProps<{
  steps: readonly WorkflowStep[]
  suggestion: string
}>()
</script>

<template>
  <section class="workflow" aria-label="AI 工作流提示">
    <ol class="workflow-steps" role="list">
      <li
        v-for="(step, index) in steps"
        :key="step.key"
        class="workflow-step"
        :class="step.state"
        :aria-current="step.state === 'active' ? 'step' : undefined"
      >
        <span class="workflow-dot" aria-hidden="true">{{ step.state === 'done' ? '✓' : index + 1 }}</span>
        <span class="workflow-label">{{ step.label }}</span>
        <span v-if="step.note" class="workflow-note tnum">{{ step.note }}</span>
        <span v-if="index < steps.length - 1" class="workflow-line" aria-hidden="true" />
      </li>
    </ol>
    <p class="workflow-suggestion" aria-live="polite">当前建议：{{ suggestion }}</p>
  </section>
</template>

<style scoped>
.workflow {
  display: flex;
  align-items: center;
  gap: 16px;
  min-height: 36px;
  padding: 0 20px;
  border-bottom: 1px solid var(--border);
  background: var(--bg-card);
}
.workflow-steps {
  display: flex;
  align-items: center;
  gap: 4px;
  min-width: 0;
  list-style: none;
  padding: 0;
}
.workflow-step {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  min-width: 0;
  padding: 4px 6px;
  border-radius: var(--radius-sm);
  color: var(--text-muted);
  font-size: 10.5px;
  white-space: nowrap;
}
.workflow-dot {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 15px;
  height: 15px;
  flex-shrink: 0;
  border: 1px solid var(--border-strong);
  border-radius: 50%;
  font-size: 9px;
}
.workflow-step.done { color: var(--text-sub); }
.workflow-step.done .workflow-dot { border-color: var(--success); color: var(--success); }
.workflow-step.active {
  color: rgb(var(--accent-rgb));
  background: var(--bg-selected);
}
.workflow-step.active .workflow-dot { border-color: rgb(var(--accent-rgb)); }
.workflow-step.failed { color: var(--danger); }
.workflow-step.failed .workflow-dot { border-color: var(--danger); }
.workflow-note { color: var(--text-muted); font-size: 10px; }
.workflow-step.active .workflow-note { color: rgb(var(--accent-rgb) / 0.8); }
.workflow-line {
  width: 12px;
  height: 1px;
  margin-left: 2px;
  background: var(--border);
}
.workflow-suggestion {
  min-width: 0;
  margin: 0 0 0 auto;
  overflow: hidden;
  color: var(--text-sub);
  font-size: 10.5px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

@media (max-width: 900px) {
  .workflow { align-items: flex-start; flex-direction: column; gap: 2px; padding: 6px 20px; }
  .workflow-suggestion { margin-left: 0; }
}
</style>
