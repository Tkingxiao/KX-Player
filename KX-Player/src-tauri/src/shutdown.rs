//! 退出收尾的统一 supervisor（P1-3）。
//!
//! 退出曾经是「广播 `window:beforeClose` → `sleep(300ms)` → `kill_all()` → `app.exit(0)`」：
//! 那 300 ms 是「大概够前端写完设置」的猜测 —— 猜短了设置丢、猜长了白等；其余环节
//! （mpv 命令线程、舞台覆盖窗口、扫描、watcher、AI、响度/转换队列）各自拆解，谁先谁后
//! 没有任何保证。同一台机器上「谁在跑、跑到哪一步」也无从查证。
//!
//! 这里按 AUDIT 给的次序收成**一条流水线**，每一步都有可观测的判据和明确的预算：
//!
//! 1. 停止接收 —— 关子进程闸门、置「不再受理长任务」、置 `force_close`（关窗即销毁，
//!    不再被拦成「隐藏到托盘」）
//! 2. 取消在飞任务 —— 扫描票作废、AI 任务取消、转换队列取消、响度分析取消
//! 3. 停文件监听 —— watcher 丢掉，worker 随之退休
//! 4. 等任务收尾 —— 等到各任务表清空（3 秒预算）
//! 5. 杀子进程 —— `fftools::kill_all()`：通知、kill、**等回收到位**（自带 5 秒上限）
//! 6. 等前端落盘 —— 广播 `window:beforeClose` 并**等 `shutdown_ready` 回执**：
//!    回执到 = `save_settings` 已经写完，而不是「睡了一会儿，大概写完了」
//! 7. 收线程 —— 关 mpv 命令队列并等命令线程退场，销毁舞台覆盖窗口
//!
//! 走出第七步之后由 [`request_exit`] 调 `app.exit(0)`。
//!
//! 为什么是「预算 + 记账」而不是「等到真的一干二净」：一个卡住的网络请求或一个卡死的
//! 子进程不该让用户关不掉程序。每一步都有上限，超了就记一条可查的日志再往下走 ——
//! 与 `kill_all` 的 5 秒上限同一取舍，只是现在整条路径的上界是**算得出来的**：
//! 3（等任务）+ 5（杀子进程）+ 2（等落盘）+ 2（收线程）+ 各步的零头。
//!
//! 单一入口：[`request_exit`]。托盘「退出」、`app_force_quit`、`force_close_window` 与
//! 「窗口全关 → Tauri 请求退出」（`RunEvent::ExitRequested`）都走它；重复的请求只跑一遍。

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

/// 等任务收尾的预算。等不到只记账 —— 卡在子进程上的任务由下一步 `kill_all` 打断。
const WAIT_TASKS_BUDGET: Duration = Duration::from_secs(3);
/// 等前端落盘回执的预算。正常路径下回执是毫秒级（一次原子写），这里是它的上限。
const FLUSH_BUDGET: Duration = Duration::from_secs(2);
/// 等 mpv 命令线程退场的预算（排队中的命令要先跑完）。
const JOIN_BUDGET: Duration = Duration::from_secs(2);
/// 轮询间隔：只在「等一件不常发生的事」上用。
const TICK: Duration = Duration::from_millis(10);

/// 收尾权：只有第一个调用者拿到。**退出只能跑一遍** —— 两个线程同时拆同一套东西
/// （一个在杀子进程、一个已经在收线程）比晚退几秒糟得多。
static EXITING: AtomicBool = AtomicBool::new(false);
/// 还受理长任务吗（扫描 / AI / 转换 / 响度 / 监听）。第一步就关掉：退出途中再拉起一个
/// 全库扫描是最坏的一种「最后动作」。
static ACCEPTING: AtomicBool = AtomicBool::new(true);
/// 前端落盘回执：`None` = 还没回，`Some(ok)` = 前端写完并如实报告了成败。
static READY: once_cell::sync::Lazy<(parking_lot::Mutex<Option<bool>>, parking_lot::Condvar)> =
    once_cell::sync::Lazy::new(|| (parking_lot::Mutex::new(None), parking_lot::Condvar::new()));

/// 收尾是否已经开始（`lib.rs` 的 `RunEvent::ExitRequested` 据此决定拦不拦退出请求）。
pub fn exiting() -> bool {
    EXITING.load(Ordering::SeqCst)
}

/// 还在受理长任务吗。各长任务的入口（扫描 / AI / 转换 / 响度 / 监听）据此当场回绝。
pub fn accepting() -> bool {
    ACCEPTING.load(Ordering::SeqCst)
}

/// 前端落盘回执（`shutdown_ready` 命令）：`ok` = 那次 `save_settings` 是否写成功。
/// 只认第一次 —— 重复的回执不该把已有的结论改掉。
pub fn mark_ready(ok: bool) {
    let (m, cv) = &*READY;
    let mut slot = m.lock();
    if slot.is_none() {
        *slot = Some(ok);
        cv.notify_all();
    }
}

/// 认领收尾权：只有第一个调用者拿到 `true`。
fn claim() -> bool {
    !EXITING.swap(true, Ordering::SeqCst)
}

/// 等前端落盘回执。`None` = 预算内没等到（调用方记日志继续走，绝不无限等）。
fn wait_ready(budget: Duration) -> Option<bool> {
    let (m, cv) = &*READY;
    let mut slot = m.lock();
    let deadline = Instant::now() + budget;
    while slot.is_none() {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return None;
        }
        cv.wait_for(&mut slot, left);
    }
    *slot
}

/// 轮询等到 `done()` 成真，或预算耗尽。收尾里所有的「等」都长这样：**有上限**。
fn wait_until(budget: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + budget;
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(TICK);
    }
}

/// 一步的收尾结果（日志与用例的记账口径）。
pub struct Outcome {
    pub name: &'static str,
    pub elapsed: Duration,
    pub panicked: bool,
}

/// 一步的收尾动作。装箱是为了让名字不同的步能放进同一个 `Vec`。
type Step = Box<dyn FnOnce() + Send>;

/// 收尾流水线：一条排好序的步骤链，逐步跑完。
pub struct Pipeline {
    steps: Vec<(&'static str, Step)>,
}

impl Default for Pipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl Pipeline {
    pub fn new() -> Self {
        Self { steps: Vec::new() }
    }

    pub fn step(mut self, name: &'static str, f: impl FnOnce() + Send + 'static) -> Self {
        self.steps.push((name, Box::new(f)));
        self
    }

    /// 按序跑完每一步。单步 panic **不中断**后面的步：收尾途中最不该做的就是半路停下 ——
    /// 已经漏做的那件事记在 `panicked` 里，后面该做的照做（退出本身绝不能漏）。
    pub fn run(self) -> Vec<Outcome> {
        let mut out = Vec::with_capacity(self.steps.len());
        for (name, f) in self.steps {
            let t0 = Instant::now();
            let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).is_err();
            let elapsed = t0.elapsed();
            if panicked {
                crate::paths::append_log(&format!("[shutdown] 步骤「{name}」panic：该步的收尾动作没做完"));
            }
            out.push(Outcome { name, elapsed, panicked });
        }
        out
    }
}

/// 退出：**唯一入口**。第一条请求跑完整条流水线，其余的当场返回。
pub fn request_exit(app: &AppHandle, reason: &'static str) {
    if !claim() {
        return;
    }
    let app = app.clone();
    // 启动失败分支要用的另一份：闭包把上面那份拿走了，这里留一手才关得掉程序
    let fallback = app.clone();
    let spawned = std::thread::Builder::new().name("shutdown".into()).spawn(move || {
        let t0 = Instant::now();
        crate::paths::append_log(&format!("[shutdown] 开始收尾（{reason}）"));
        let steps: Vec<String> = build_pipeline(app.clone())
            .run()
            .iter()
            .map(|o| format!("{} {}{}ms", o.name, if o.panicked { "panic " } else { "" }, o.elapsed.as_millis()))
            .collect();
        crate::paths::append_log(&format!("[shutdown] 收尾完成（{}），共 {} ms，退出", steps.join(" · "), t0.elapsed().as_millis()));
        app.exit(0);
    });
    if let Err(e) = spawned {
        // 收尾线程都起不来（极罕见）：也得把程序关掉 —— 用户不该因为收尾失败而关不掉它
        crate::paths::append_log(&format!("[shutdown] 收尾线程启动失败（{e}），直接退出"));
        fallback.exit(0);
    }
}

fn build_pipeline(app: AppHandle) -> Pipeline {
    let (a1, a2, a3, a4) = (app.clone(), app.clone(), app.clone(), app.clone());
    Pipeline::new()
        .step("停止接收", move || stop_accepting(&a1))
        .step("取消在飞任务", cancel_in_flight)
        .step("停文件监听", move || stop_file_watching(&a2))
        .step("等任务收尾", wait_tasks)
        .step("杀子进程", crate::fftools::kill_all)
        .step("等前端落盘", move || flush_frontend(&a3))
        .step("收线程", move || join_background(&a4))
}

/// 第一步：不再受理新活。置位之后 `register` 一律失败（不再出生 ffmpeg/ffprobe），
/// 各长任务入口当场回绝，主窗口的关闭请求不再被拦成「隐藏到托盘」。
fn stop_accepting(app: &AppHandle) {
    ACCEPTING.store(false, Ordering::SeqCst);
    crate::fftools::close_gate();
    if let Some(state) = app.try_state::<crate::state::AppWindowsState>() {
        state.force_close.store(true, Ordering::SeqCst);
    }
}

/// 第二步：取消在飞的活。只置取消位，任务各自在检查点上收手 ——
/// 还卡在子进程上的那些由第五步 `kill_all` 打断。
fn cancel_in_flight() {
    let scans = crate::scanner::gate::GATE.cancel_all();
    let ai = crate::ai_job::cancel_all();
    let convert = crate::fftools::cancel_all_convert();
    let loudness = crate::commands::library::request_cancel_loudness();
    crate::paths::append_log(&format!(
        "[shutdown] 已取消：扫描 {scans} 个在飞票据、AI {ai} 个任务、转换 {convert} 条队列、响度{}",
        if loudness { "有在跑" } else { "没在跑" }
    ));
}

/// 第三步：停文件监听。`stop` 换代际并丢掉 watcher，worker 随之退休（丢掉的那一批不发）。
fn stop_file_watching(app: &AppHandle) {
    if let Some(state) = app.try_state::<crate::state::AppWindowsState>() {
        crate::watcher::stop(&state.watcher);
    }
}

/// 第四步：等在飞任务收尾。等不到只记账 —— 还在跑的多半卡在网络（AI）或子进程
/// （转换/响度）上，前者由预算兜底、后者由下一步 `kill_all` 打断。
fn wait_tasks() {
    wait_until(WAIT_TASKS_BUDGET, || pending_tasks().is_empty());
    let pending = pending_tasks();
    if !pending.is_empty() {
        crate::paths::append_log(&format!("[shutdown] 等任务收尾到点，仍在跑：{}（不阻塞退出）", pending.join("、")));
    }
}

/// 还没收尾的任务（日志口径）。
fn pending_tasks() -> Vec<String> {
    let mut out = Vec::new();
    let scans = crate::scanner::gate::GATE.live_count();
    if scans > 0 {
        out.push(format!("扫描×{scans}"));
    }
    let ai = crate::ai_job::running();
    if ai > 0 {
        out.push(format!("AI×{ai}"));
    }
    let convert = crate::fftools::convert_tasks();
    if convert > 0 {
        out.push(format!("转换×{convert}"));
    }
    if crate::commands::library::loudness_running() {
        out.push("响度×1".to_string());
    }
    out
}

/// 第六步：等前端把设置写进盘。回执来自 `shutdown_ready` 命令 —— 它在前端的
/// `save_settings` 真的返回之后才发，所以「等到回执」= 「已经落盘」，不是猜。
fn flush_frontend(app: &AppHandle) {
    if app.get_webview_window("main").is_none() {
        crate::paths::append_log("[shutdown] 主窗口已销毁，跳过前端落盘（设置停在最后一次自动保存）");
        return;
    }
    if let Err(e) = app.emit("window:beforeClose", ()) {
        crate::paths::append_log(&format!("[shutdown] 落盘广播失败：{e}"));
        return;
    }
    match wait_ready(FLUSH_BUDGET) {
        Some(true) => crate::paths::append_log("[shutdown] 前端已确认设置落盘"),
        Some(false) => crate::paths::append_log("[shutdown] 前端报告设置没写成功（回执已到，继续退出）"),
        None => crate::paths::append_log("[shutdown] 等不到前端落盘回执，继续退出（设置停在最后一次自动保存）"),
    }
}

/// 第七步：收线程。关 mpv 命令队列并等命令线程真的退场（排队中的命令先跑完），
/// 顺带销毁舞台覆盖窗口。从没播过（没起过 mpv）时这里是空转。
fn join_background(app: &AppHandle) {
    if let Some(player) = app.try_state::<crate::PlayerCore>() {
        if !player.shutdown(JOIN_BUDGET) {
            crate::paths::append_log("[shutdown] mpv 命令线程未在预算内退场，继续退出");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 流水线用例都动全局闸门/回执：串行跑，跑完各自复位（同进程里还有别的用例在读它们）。
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static L: std::sync::Mutex<()> = std::sync::Mutex::new(());
        L.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn reset() {
        EXITING.store(false, Ordering::SeqCst);
        ACCEPTING.store(true, Ordering::SeqCst);
        *READY.0.lock() = None;
    }

    #[test]
    fn steps_run_in_the_documented_order() {
        let seen: std::sync::Arc<parking_lot::Mutex<Vec<&'static str>>> = Default::default();
        let (a, b, c) = (seen.clone(), seen.clone(), seen.clone());
        let out = Pipeline::new()
            .step("停止接收", move || a.lock().push("停止接收"))
            .step("取消在飞任务", move || b.lock().push("取消在飞任务"))
            .step("收线程", move || c.lock().push("收线程"))
            .run();
        assert_eq!(*seen.lock(), vec!["停止接收", "取消在飞任务", "收线程"], "次序就是 AUDIT 给的次序");
        assert_eq!(out.iter().map(|o| o.name).collect::<Vec<_>>(), vec!["停止接收", "取消在飞任务", "收线程"]);
        assert!(out.iter().all(|o| !o.panicked));
    }

    /// 单步 panic 不该让后面的步被跳过 —— 退出本身绝不能因为某一步坏了而漏掉。
    #[test]
    fn a_panicking_step_does_not_stop_the_rest() {
        let after = std::sync::Arc::new(AtomicBool::new(false));
        let flag = after.clone();
        let out = Pipeline::new()
            .step("坏掉的一步", || panic!("这一步炸了"))
            .step("后面的一步", move || flag.store(true, Ordering::SeqCst))
            .run();
        assert!(out[0].panicked, "炸了要如实记账");
        assert!(!out[1].panicked);
        assert!(after.load(Ordering::SeqCst), "后面的步照跑");
    }

    /// 退出只能跑一遍：重复的请求（托盘点了两次、窗口关闭又叠一个退出请求）直接返回。
    #[test]
    fn only_the_first_claim_runs_the_pipeline() {
        let _g = serial();
        reset();
        assert!(claim(), "第一个调用者拿到收尾权");
        assert!(exiting());
        assert!(!claim(), "第二个调用者当场返回");
        assert!(accepting(), "收尾权只说不许再收尾，受理闸门由第一步自己关");
        reset();
        assert!(!exiting());
        assert!(accepting());
    }

    /// 收尾里的每一次「等」都是有上限的：等不到就到点走人，不是无限等。
    #[test]
    fn a_wait_without_a_signal_gives_up_at_the_budget() {
        let t0 = Instant::now();
        assert!(!wait_until(Duration::from_millis(80), || false));
        let took = t0.elapsed();
        assert!(took >= Duration::from_millis(80), "不许提前放弃：{took:?}");
        assert!(took < Duration::from_secs(2), "也不许超过预算太久：{took:?}");
    }

    /// 判据成真就当场放行，不等满预算（这正是它比 sleep 好的地方）。
    #[test]
    fn a_wait_is_released_as_soon_as_the_signal_arrives() {
        let ready = std::sync::Arc::new(AtomicBool::new(false));
        let flag = ready.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(40));
            flag.store(true, Ordering::SeqCst);
        });
        let t0 = Instant::now();
        assert!(wait_until(Duration::from_secs(5), || ready.load(Ordering::SeqCst)));
        assert!(t0.elapsed() < Duration::from_millis(900), "回执/判据一到就该放行：{:?}", t0.elapsed());
    }

    /// 前端的落盘回执一到，第六步立刻放行；回执带的是「成没写成」这个事实。
    #[test]
    fn the_frontend_ack_releases_the_flush_wait_with_its_verdict() {
        let _g = serial();
        reset();
        let t = std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(40));
            mark_ready(false);
        });
        let t0 = Instant::now();
        assert_eq!(wait_ready(Duration::from_secs(5)), Some(false), "回执照实转达：这次没写成功");
        assert!(t0.elapsed() < Duration::from_millis(900), "回执一到就该放行：{:?}", t0.elapsed());
        t.join().unwrap();
        // 重复回执不改结论
        mark_ready(true);
        assert_eq!(wait_ready(Duration::from_millis(1)), Some(false));
        reset();
    }

    /// 等不到回执 = `None`：调用方据此记「设置停在最后一次自动保存」，而不是假装写完了。
    #[test]
    fn no_ack_means_no_verdict() {
        let _g = serial();
        reset();
        let t0 = Instant::now();
        assert_eq!(wait_ready(Duration::from_millis(60)), None);
        assert!(t0.elapsed() >= Duration::from_millis(60));
        reset();
    }
}
