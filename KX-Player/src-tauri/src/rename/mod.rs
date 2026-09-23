//! 改名主干（`04 §7`）：判定要不要动 → 本地规则能修的先修 → 剩下的才留给模型。
//!
//! 读写盘的分界：`norm`/`judge`/`postcheck` 是纯函数，`preview` 只多了一处列目录，`history` 的
//! 合并逻辑收的是 `serde_json::Value`（不落盘）。真正**改文件系统**的只有 `apply` + `rollback`
//! （顺序与换算在 `paths`）—— 这样拆分的理由是判定的正确性只能靠用例证明，而一次写错的批量
//! 改名没有第二次机会。
//!
//! 模型那一侧拆成四格：`ai_name` 是契约（送什么、怎么读回来），`postcheck` 是那九道校验，
//! `ai_ask` 是与端点往来的流水，`ai_ledger` 是译名账本（哪些能取、谁被拒过）。**落地**永远由
//! `apply` 重算预览、按片段查库之后决定（`04 §7.7`）—— 这一格拆开是有代价的（四个模块要互相
//! 认账），换来的是「烧过 token 的结果」和「改过文件名的动作」之间存在一个能查证的中间层。

pub mod ai_ask;
pub mod ai_ledger;
pub mod ai_name;
pub mod apply;
pub mod batch;
pub mod history;
pub mod judge;
pub mod listing;
pub mod norm;
pub mod paths;
pub mod postcheck;
pub mod preview;
pub mod report;
pub mod rollback;
