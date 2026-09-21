/**
 * 卡片与列表行的几何常量。CSS 侧同名值在 styles/tokens.css（--card-row-h / --row-h），
 * 这里改了就不同步 —— 两处一起改。虚拟化网格需要 JS 数值，故不能只留 CSS 变量。
 */

/** 封面之下信息区占用的像素高度（内边距 + 边框 + 标题行 + 副标题行） */
export const CARD_CHROME_H = 56

/** 网格行高 = 封面边长 + 信息区；默认 gridSize 176 → 232px（03 §2.1 硬指标） */
export function cardRowHeight(gridSize: number): number {
  return gridSize + CARD_CHROME_H
}

/** 列表行高，对应 tokens.css 的 --row-h */
export const ROW_H = 46
