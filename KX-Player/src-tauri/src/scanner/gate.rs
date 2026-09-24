//! 扫描闸门（P1-1）：同一时刻只认最新一次扫描，旧的要么被取代、要么落库被拦。
//!
//! 为什么是全局一张闸门、而不是按 root 各管一段：`save_snapshot` 写的是**整库**差异 ——
//! 它会把库里「本轮没见到」的曲目删掉。两个根同时扫，后落库的那个会把另一个根的曲目
//! 全部当成「没见到」。所以互斥必须是全局的：只有最新那次扫描有落库权。
//!
//! 语义：
//! - `begin()` 收回此前所有在飞的票（把它们标记为 superseded），并发出一张新票；
//! - 票的主人可以在长任务里随时用 `superseded()` 提前收手（省 CPU），
//!   但真正的红线是 `if_current()` —— 落库与「谁是最新」在同一把锁里判，旧任务绝不允许覆盖新任务；
//! - `finish()` 交还票，防止表里堆积。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// 一次扫描的票。`Send`，可以随 `spawn_blocking` 一起进到工作线程。
#[derive(Clone)]
pub struct Ticket {
    superseded: Arc<AtomicBool>,
}

impl Ticket {
    /// 是否已被更新的扫描取代（长任务用它提前收手）。
    pub fn superseded(&self) -> bool {
        self.superseded.load(Ordering::Acquire)
    }

    /// 取消旗子的副本，给并行解析池这类「只能拿到裸旗子」的地方用。
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.superseded)
    }
}

#[derive(Default)]
pub struct ScanGate {
    live: Mutex<Vec<Arc<AtomicBool>>>,
}

impl ScanGate {
    /// 领票并取代此前所有在飞的扫描。
    pub fn begin(&self) -> Ticket {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        for prev in live.drain(..) {
            prev.store(true, Ordering::Release);
        }
        let flag = Arc::new(AtomicBool::new(false));
        live.push(Arc::clone(&flag));
        Ticket { superseded: flag }
    }

    /// 交还票。已经被取代（或已被清理）的票是幂等的 no-op。
    pub fn finish(&self, ticket: &Ticket) {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        live.retain(|f| !Arc::ptr_eq(f, &ticket.superseded));
    }

    /// 持着闸门锁跑一段「只有最新那次扫描才有权做」的收尾（落库）。
    ///
    /// 持锁期间新扫描的 `begin()` 会阻塞 —— 这正是要的：判「谁是最新」与真正写库
    /// 必须合成一个不可分割的动作，否则两者之间挤进来的新扫描仍可能被旧的覆盖。
    pub fn if_current<R>(&self, ticket: &Ticket, f: impl FnOnce() -> R) -> Option<R> {
        let live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        if !live.iter().any(|flag| Arc::ptr_eq(flag, &ticket.superseded)) {
            return None;
        }
        Some(f())
    }

    /// 退出用（P1-3）：把在飞的票全部标记为作废 —— 与「被新的扫描取代」同一效果，
    /// 但**票留在表里**，由各自的主人 `finish()` 交还。[`live_count`] 因此能一直观察到
    /// 「真的没人跑了」，而不是「我标记完了就当它停了」。
    pub fn cancel_all(&self) -> usize {
        let live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        for flag in live.iter() {
            flag.store(true, Ordering::Release);
        }
        live.len()
    }

    /// 在飞的扫描条数（退出收尾等它归零的判据）。
    pub fn live_count(&self) -> usize {
        self.live.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

/// 进程级闸门：曲库只有一份，扫描的落库权也就只有一张票。
pub static GATE: once_cell::sync::Lazy<ScanGate> = once_cell::sync::Lazy::new(ScanGate::default);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_ticket_is_current() {
        let gate = ScanGate::default();
        let t = gate.begin();
        assert!(!t.superseded());
        assert!(!t.superseded());
    }

    #[test]
    fn a_new_scan_supersedes_the_previous_one() {
        let gate = ScanGate::default();
        let old = gate.begin();
        let new = gate.begin();
        assert!(old.superseded(), "旧票必须被收回");
        assert!(old.superseded(), "旧任务不允许再落库");
        assert!(!new.superseded(), "新票必须是当前");
    }

    #[test]
    fn the_latest_scan_survives_older_ones_finishing() {
        let gate = ScanGate::default();
        let old = gate.begin();
        let new = gate.begin();
        gate.finish(&old);
        assert!(!new.superseded(), "旧任务退场不该影响最新那张票");
    }

    #[test]
    fn finishing_a_stale_ticket_does_not_disturb_the_current_one() {
        let gate = ScanGate::default();
        let old = gate.begin();
        let new = gate.begin();
        gate.finish(&old);
        gate.finish(&old); // 幂等
        let newest = gate.begin();
        assert!(new.superseded());
        assert!(!newest.superseded());
        assert_eq!(gate.live.lock().unwrap_or_else(|e| e.into_inner()).len(), 1, "交还的票不该留在表里");
    }

    #[test]
    fn only_the_current_ticket_may_run_the_commit_step() {
        let gate = ScanGate::default();
        let old = gate.begin();
        assert_eq!(gate.if_current(&old, || "写了"), Some("写了"), "最新的票有权落库");
        let _new = gate.begin();
        assert_eq!(gate.if_current(&old, || "写了"), None, "被取代的票一步都不许写");
    }

    #[test]
    fn a_commit_blocks_a_new_scan_from_being_declared_current() {
        // 落库与「谁是最新」必须在同一把锁里：写库期间 begin() 进不来，
        // 于是「先判后写」中间不会插进一个新扫描被旧结果覆盖。
        let gate = ScanGate::default();
        let t = gate.begin();
        gate.if_current(&t, || {
            assert!(gate.live.try_lock().is_err(), "落库期间闸门必须处于持有状态");
        });
    }

    #[test]
    fn the_cancel_flag_mirrors_the_ticket() {
        let gate = ScanGate::default();
        let t = gate.begin();
        let flag = t.cancel_flag();
        assert!(!flag.load(Ordering::Relaxed));
        let _new = gate.begin();
        assert!(flag.load(Ordering::Relaxed), "取消旗子要能让 worker 看见");
    }

    /// 退出收尾（P1-3）靠这两个方法判断「扫描真的停了」：`cancel_all` 只作废、
    /// 不摘表 —— 摘表就等于宣布「它停了」，而任务可能还在解析。
    #[test]
    fn cancel_all_invalidates_every_live_ticket_and_live_count_tracks_them() {
        let gate = ScanGate::default();
        assert_eq!(gate.live_count(), 0, "闲时没有在飞票据");

        let old = gate.begin();
        let current = gate.begin();
        assert!(!current.superseded());
        assert_eq!(gate.cancel_all(), 1, "begin 已经收回了旧票，表里只剩最新的那张");
        assert!(old.superseded());
        assert!(current.superseded(), "在飞的那张也要作废");
        assert_eq!(gate.live_count(), 1, "票还留在表里：要等主人自己交还才算停");

        gate.finish(&current);
        assert_eq!(gate.live_count(), 0);
    }
}
