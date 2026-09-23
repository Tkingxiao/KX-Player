/**
 * 改名面板的文案表：档位、命中原因、冲突类型 → 中文。
 *
 * 判定全在 Rust（`src-tauri/src/rename/`），这里只负责「说人话」。三份都是**穷举映射**：
 * Rust 侧新增一档 / 一个原因而这里没跟上，`vue-tsc` 会直接报缺键，而不是让人看到 `undefined`。
 */
import type { RenameConflict, RenameReason, RenameVerdict } from '@/contracts/api'

export const VERDICT_LABEL: Record<RenameVerdict, string> = {
  skip: '跳过',
  normalize_only: '只需规范化',
  needs_repair: '要修',
  needs_translation: '要翻',
  compliant: '已合规',
}

export const REASON_LABEL: Record<RenameReason, string> = {
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

export const CONFLICT_LABEL: Record<RenameConflict, string> = {
  same_target: '这批里撞名',
  target_exists: '目标已存在',
  path_too_long: '全路径超长',
}

/** 绝对路径 → 末段名字。库里的路径两种分隔符都可能出现过，两个都当切点 */
export const base = (p: string): string => p.split(/[\\/]/).pop() ?? p
