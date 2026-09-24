//! mpv 单一命令线程 + 有界命令队列（P1-2）。
//!
//! libmpv 的 `set_property` / `command` 是线程安全的，但「线程安全」不等于「有序」：
//! 好几个 Tauri 命令线程加上事件线程一起下发时，谁先到 mpv 由调度决定。连点两次播放、
//! 拖音量时连发几十条 `volume`、换曲与暂停交错，都可能以意外的顺序落到 mpv 上 ——
//! 而 mpv 只会按收到的顺序执行。
//!
//! 这里把**所有** mpv 访问收进一条命令线程：调用方投作业，线程按 FIFO 依次执行。
//! 队列有硬上限，满了就等（等到超时报错，不无声丢弃）；同一个键（`Some("volume")`
//! 这类绝对量属性）在排队期间只留最后一条 —— 连发既不堆队列，也不会拿旧值覆盖新值，
//! 真正执行的那条永远是最后一次意图。
//!
//! 泛型参数 `C` 是命令作用的目标（生产环境是 `Mpv`）。测试拿假上下文驱动同一条队列，
//! 顺序、合并、上限、panic 隔离这些性质不必启动真 mpv 就能钉住。

use std::collections::VecDeque;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::error::{AppErrorCode, IpcError, IpcResult};

/// 排队上限。给正常连发留足余地，又不至于让「卡住的 mpv」无限吃内存。
pub const DEFAULT_CAP: usize = 32;
/// 队列满时投递方最多等这么久（等到还满就报拥堵，让 UI 看到原因而不是无限等）。
const DEFAULT_PUSH_TIMEOUT: Duration = Duration::from_secs(5);
/// 等一条命令执行完的上限。mpv 的正常调用在毫秒级，超过这个数说明引擎那边出了问题。
const DEFAULT_REPLY_TIMEOUT: Duration = Duration::from_secs(10);
/// 空闲轮询间隔：只用于「线程退出」这类不常发生的事，不参与正常调度。
const IDLE_TICK: Duration = Duration::from_millis(200);

thread_local! {
    /// 命令线程自己的标记：作业里再投作业会等自己执行完（自锁），必须当场拦住。
    static IN_WORKER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

type Job<C> = Box<dyn FnOnce(&C) + Send + 'static>;

struct Pending<C> {
    /// 合并键：同键的排队作业只留最后一条；`None` = 不可合并（带先后语义的命令）
    key: Option<&'static str>,
    run: Job<C>,
}

struct State<C> {
    jobs: VecDeque<Pending<C>>,
    /// 已关停：不再收新作业（排队中的照常跑完）
    dead: bool,
}

pub struct CommandQueue<C> {
    state: Mutex<State<C>>,
    /// 有新作业可跑
    work: Condvar,
    /// 有空位了
    space: Condvar,
    /// 命令线程是否已经退场（退出收尾等它 join 的判据，见 `wait_stopped`）
    stopped: Arc<AtomicBool>,
    cap: usize,
    push_timeout: Duration,
    reply_timeout: Duration,
}

impl<C: Send + Sync + 'static> CommandQueue<C> {
    /// 起一条命令线程。`ctx` 归它独占（生产环境里就是那份 `Arc<Mpv>` 的持有者）。
    pub fn start(ctx: C) -> IpcResult<Arc<Self>> {
        Self::start_with(ctx, DEFAULT_CAP, DEFAULT_PUSH_TIMEOUT, DEFAULT_REPLY_TIMEOUT)
    }

    /// 可调参数的构造点：测试用小上限/短超时，避免为一两条断言干等几十秒。
    pub fn start_with(ctx: C, cap: usize, push_timeout: Duration, reply_timeout: Duration) -> IpcResult<Arc<Self>> {
        let q = Arc::new(Self {
            state: Mutex::new(State { jobs: VecDeque::new(), dead: false }),
            work: Condvar::new(),
            space: Condvar::new(),
            stopped: Arc::new(AtomicBool::new(false)),
            cap: cap.max(1),
            push_timeout,
            reply_timeout,
        });
        let worker = Arc::clone(&q);
        std::thread::Builder::new()
            .name("mpv-cmd".into())
            .spawn(move || worker_loop(worker, ctx))
            .map_err(|e| IpcError::with_detail(AppErrorCode::MpvInitFailed, "播放器命令线程启动失败", e))?;
        Ok(q)
    }

    /// 投一条命令并等它执行完（同步语义：调用方拿得到 mpv 的真实结果）。
    ///
    /// `key` 是合并键：同一键的作业在**排队期间**只保留最后一条。正在执行的那条不受影响
    /// （它已经脱离队列），所以「合并」只会省掉还没轮到的中间值 —— 那正是连发里多余的部分。
    pub fn submit<F>(&self, key: Option<&'static str>, f: F) -> IpcResult<()>
    where
        F: FnOnce(&C) -> IpcResult<()> + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        self.push(key, Box::new(move |ctx| { let _ = tx.send(f(ctx)); }))?;
        match rx.recv_timeout(self.reply_timeout) {
            Ok(r) => r,
            Err(RecvTimeoutError::Disconnected) => Err(worker_gone()),
            Err(RecvTimeoutError::Timeout) => Err(IpcError::with_detail(
                AppErrorCode::Internal,
                "播放器命令超时",
                format!("{:?} 内没有执行完", self.reply_timeout),
            )),
        }
    }

    /// 读一条 mpv 属性并等结果（如字幕轨、输出设备清单）。
    /// 读取也走同一条线程：一边 loadfile 一边读 track-list 拿到的是半截状态。
    pub fn query<T, F>(&self, f: F) -> IpcResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&C) -> T + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        self.push(None, Box::new(move |ctx| { let _ = tx.send(f(ctx)); }))?;
        match rx.recv_timeout(self.reply_timeout) {
            Ok(v) => Ok(v),
            Err(RecvTimeoutError::Disconnected) => Err(worker_gone()),
            Err(RecvTimeoutError::Timeout) => Err(IpcError::with_detail(
                AppErrorCode::Internal,
                "播放器命令超时",
                format!("{:?} 内没有执行完", self.reply_timeout),
            )),
        }
    }

    /// 不等结果的投递（事件线程用：它还要继续收事件，不能被一条 mpv 调用扣住）。
    /// 队列满或线程已退出时当场返回 `Err`，由调用方决定记不记日志 —— 不排队、不阻塞。
    pub fn post<F>(&self, key: Option<&'static str>, f: F) -> IpcResult<()>
    where
        F: FnOnce(&C) -> IpcResult<()> + Send + 'static,
    {
        self.try_push(key, Box::new(move |ctx| { let _ = f(ctx); }))
    }

    /// 关停：不再收新作业，排队中的照常跑完，然后线程退出。
    pub fn shutdown(&self) {
        let mut st = self.lock();
        if st.dead {
            return;
        }
        st.dead = true;
        self.work.notify_all();
    }

    /// 等命令线程真的退场（P1-3 的「收线程」一步）。关停后排队中的作业会先跑完再退，
    /// 所以这里等的是「活干完 + 线程退出」；到点没退只回 `false`，绝不无限等。
    pub fn wait_stopped(&self, budget: Duration) -> bool {
        let deadline = Instant::now() + budget;
        while !self.stopped.load(Ordering::SeqCst) {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        true
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State<C>> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn push(&self, key: Option<&'static str>, run: Job<C>) -> IpcResult<()> {
        let mut st = self.lock();
        self.check_pushable(&st)?;
        coalesce(&mut st, key);
        let deadline = Instant::now() + self.push_timeout;
        while st.jobs.len() >= self.cap {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(self.congested(st.jobs.len()));
            }
            let (next, _) = self.space.wait_timeout(st, left).unwrap_or_else(|e| e.into_inner());
            st = next;
            if st.dead {
                return Err(worker_gone());
            }
        }
        st.jobs.push_back(Pending { key, run });
        self.work.notify_one();
        Ok(())
    }

    fn try_push(&self, key: Option<&'static str>, run: Job<C>) -> IpcResult<()> {
        let mut st = self.lock();
        self.check_pushable(&st)?;
        coalesce(&mut st, key);
        if st.jobs.len() >= self.cap {
            return Err(self.congested(st.jobs.len()));
        }
        st.jobs.push_back(Pending { key, run });
        self.work.notify_one();
        Ok(())
    }

    fn check_pushable(&self, st: &State<C>) -> IpcResult<()> {
        if st.dead {
            return Err(worker_gone());
        }
        if IN_WORKER.with(std::cell::Cell::get) {
            // 作业里等自己执行完 = 自锁。宁可报错也不能挂住命令线程。
            return Err(IpcError::new(AppErrorCode::Internal, "播放器命令线程内不得再投递命令"));
        }
        Ok(())
    }

    fn congested(&self, queued: usize) -> IpcError {
        IpcError::with_detail(
            AppErrorCode::Busy,
            "播放器命令队列拥堵",
            format!("排队 {queued} 条已达上限 {}", self.cap),
        )
    }
}

/// 把新作业的键上已有的排队作业换掉（latest-wins）：一条绝对量属性下发十次，
/// 前面的九次在执行前就已经过时了。
fn coalesce<C>(st: &mut State<C>, key: Option<&'static str>) {
    if let Some(k) = key {
        st.jobs.retain(|p| p.key != Some(k));
    }
}

fn worker_gone() -> IpcError {
    IpcError::new(AppErrorCode::Internal, "播放器命令线程已退出")
}

fn worker_loop<C: Send + Sync + 'static>(q: Arc<CommandQueue<C>>, ctx: C) {
    loop {
        let job = {
            let mut st = q.lock();
            loop {
                if let Some(job) = st.jobs.pop_front() {
                    q.space.notify_one(); // 仍持锁：等待空位的一方不会丢唤醒
                    break Some(job);
                }
                if st.dead {
                    break None;
                }
                let (next, _) = q.work.wait_timeout(st, IDLE_TICK).unwrap_or_else(|e| e.into_inner());
                st = next;
            }
        };
        match job {
            Some(j) => {
                IN_WORKER.with(|f| f.set(true));
                // 单条命令 panic 不该带走整条线程：接住、记日志，继续服务后面的命令。
                // 该作业的回复通道随 panic 一起断开，投递方立刻拿到「线程已退出」而不是干等超时。
                let panicked = std::panic::catch_unwind(AssertUnwindSafe(|| (j.run)(&ctx))).is_err();
                IN_WORKER.with(|f| f.set(false));
                if panicked {
                    crate::paths::append_log("[mpv] 命令执行 panic：已接住，命令线程继续");
                }
            }
            None => {
                // 记账：等「线程退了吗」的一方（退出收尾）等的就是这一行
                q.stopped.store(true, Ordering::SeqCst);
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    /// 假上下文：只记执行顺序，不碰 mpv。
    #[derive(Clone)]
    struct Ctx {
        log: Arc<StdMutex<Vec<String>>>,
    }

    struct Harness {
        q: Arc<CommandQueue<Ctx>>,
        log: Arc<StdMutex<Vec<String>>>,
    }

    impl Harness {
        fn new() -> Self {
            Self::with_limits(DEFAULT_CAP, Duration::from_millis(50), Duration::from_secs(2))
        }

        fn with_limits(cap: usize, push: Duration, reply: Duration) -> Self {
            let log = Arc::new(StdMutex::new(Vec::new()));
            let q = CommandQueue::start_with(Ctx { log: Arc::clone(&log) }, cap, push, reply).expect("起命令线程");
            Self { q, log }
        }

        /// 投一条「占住命令线程」的作业并等它**真的开始执行**：占位作业一旦开跑就脱离了队列，
        /// 后续作业是排队还是拥堵才是确定的（不等这一步，占位作业可能还占着队列里的位子）。
        fn blocker(&self) -> std::sync::mpsc::Sender<()> {
            let (release_tx, release_rx) = mpsc::channel::<()>();
            let (started_tx, started_rx) = mpsc::channel::<()>();
            self.q
                .post(None, move |ctx| {
                    let _ = started_tx.send(());
                    let _ = release_rx.recv();
                    ctx.log.lock().unwrap().push("blocker".into());
                    Ok(())
                })
                .expect("占位作业");
            started_rx.recv_timeout(Duration::from_secs(2)).expect("占位作业应当开始执行");
            release_tx
        }

        fn keyed(&self, key: &'static str, value: &str) -> IpcResult<()> {
            let v = value.to_string();
            self.q.post(Some(key), move |ctx| {
                ctx.log.lock().unwrap().push(v);
                Ok(())
            })
        }

        /// 等队列把已投的作业跑完：投一条不可合并的哨兵并等它回来。
        fn drain(&self) {
            let (tx, rx) = mpsc::channel::<()>();
            self.q
                .post(None, move |_| {
                    let _ = tx.send(());
                    Ok(())
                })
                .expect("哨兵作业");
            rx.recv_timeout(Duration::from_secs(2)).expect("队列应当跑完");
        }

        fn lines(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
    }

    #[test]
    fn jobs_run_in_the_order_they_were_submitted() {
        // 深队列：200 条一次投满，跑的次序必须与投的次序一致
        let h = Harness::with_limits(256, Duration::from_millis(50), Duration::from_secs(2));
        for i in 0..200 {
            let v = format!("j{i}");
            h.q.post(None, move |ctx| {
                ctx.log.lock().unwrap().push(v);
                Ok(())
            })
            .expect("投递");
        }
        h.drain();
        let lines = h.lines();
        assert_eq!(lines.len(), 200, "一条都不许少（哨兵自己不留痕迹）");
        assert_eq!(lines[0], "j0");
        assert_eq!(lines[199], "j199");
    }

    #[test]
    fn a_queued_job_with_the_same_key_is_replaced_by_the_newest() {
        let h = Harness::new();
        let gate = h.blocker();
        for v in ["vol=10", "vol=40", "vol=70"] {
            h.keyed("volume", v).expect("投递");
        }
        gate.send(()).expect("放行");
        h.drain();
        assert_eq!(h.lines(), vec!["blocker", "vol=70"], "只执行最后一条，中间值在执行前已过时");
    }

    #[test]
    fn different_keys_do_not_coalesce_each_other() {
        let h = Harness::new();
        let gate = h.blocker();
        h.keyed("volume", "vol=70").expect("投递");
        h.keyed("speed", "speed=1.5").expect("投递");
        h.keyed("sub-pos", "pos=80").expect("投递");
        gate.send(()).expect("放行");
        h.drain();
        assert_eq!(h.lines(), vec!["blocker", "vol=70", "speed=1.5", "pos=80"], "不同属性各留一条");
    }

    #[test]
    fn unkeyed_jobs_are_never_coalesced() {
        let h = Harness::new();
        let gate = h.blocker();
        for i in 0..5 {
            let v = format!("play{i}");
            h.q.post(None, move |ctx| {
                ctx.log.lock().unwrap().push(v);
                Ok(())
            })
            .expect("投递");
        }
        gate.send(()).expect("放行");
        h.drain();
        assert_eq!(
            h.lines(),
            vec!["blocker", "play0", "play1", "play2", "play3", "play4"],
            "换曲这类有先后语义的命令一条都不许少"
        );
    }

    #[test]
    fn a_full_queue_reports_congestion_instead_of_dropping_silently() {
        let h = Harness::with_limits(2, Duration::from_millis(20), Duration::from_secs(2));
        let gate = h.blocker();
        h.q.post(None, |_| Ok(())).expect("占第 1 个位");
        h.q.post(None, |_| Ok(())).expect("占第 2 个位");
        let err = h.q.post(None, |_| Ok(())).expect_err("第 3 条应当当场报拥堵");
        assert!(err.to_string().contains("拥堵"), "{err}");
        gate.send(()).expect("放行");
    }

    #[test]
    fn a_reply_is_always_the_real_result() {
        let h = Harness::new();
        h.q.submit(None, |_| Ok(())).expect("成功的命令");
        let err = h.q
            .submit(None, |_| Err(IpcError::new(AppErrorCode::Internal, "mpv 说不行")))
            .expect_err("失败的命令");
        assert!(err.to_string().contains("mpv 说不行"), "{err}");
    }

    #[test]
    fn a_panicking_job_does_not_take_the_thread_down() {
        let h = Harness::new();
        let err = h.q.submit(None, |_| panic!("作业里炸了")).expect_err("panic 的作业要如实报错");
        assert!(err.to_string().contains("命令线程已退出"), "{err}");
        h.q.submit(None, |ctx| {
            ctx.log.lock().unwrap().push("still alive".into());
            Ok(())
        })
        .expect("命令线程应当继续服务");
        assert_eq!(h.lines(), vec!["still alive"]);
    }

    #[test]
    fn a_job_cannot_submit_more_work_to_itself() {
        let h = Harness::new();
        let inner = Arc::clone(&h.q);
        let (tx, rx) = mpsc::channel::<String>();
        h.q.submit(None, move |_| {
            let e = inner.submit(None, |_| Ok(())).expect_err("自锁投递要当场拒绝");
            let _ = tx.send(e.to_string());
            Ok(())
        })
        .expect("外层命令正常返回");
        let msg = rx.recv_timeout(Duration::from_secs(1)).expect("内层结果");
        assert!(msg.contains("不得再投递"), "{msg}");
    }

    #[test]
    fn a_read_query_goes_through_the_same_thread_and_returns_its_value() {
        let h = Harness::new();
        let gate = h.blocker();
        let (tx, rx) = mpsc::channel::<usize>();
        let inner = Arc::clone(&h.q);
        std::thread::spawn(move || {
            let n = inner.query(|ctx| ctx.log.lock().unwrap().len()).expect("查询");
            let _ = tx.send(n);
        });
        // 查询排在占位作业之后：命令线程此时还没跑它，读到的必然是「执行那一刻」的长度
        std::thread::sleep(Duration::from_millis(50));
        gate.send(()).expect("放行");
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).expect("查询结果"), 1, "读到的是占位作业写下的那一条");
    }

    #[test]
    fn a_shut_down_queue_keeps_its_promises_and_refuses_new_work() {
        let h = Harness::new();
        let gate = h.blocker();
        h.keyed("volume", "vol=70").expect("投递");
        gate.send(()).expect("放行");
        h.q.shutdown();
        let err = h.q.post(None, |_| Ok(())).expect_err("关停后不再收新作业");
        assert!(err.to_string().contains("已退出"), "错误文本应可读：{err}");
        // 关停前排队的作业仍然跑完（不是「关停即丢」）
        let deadline = Instant::now() + Duration::from_secs(2);
        while h.lines().len() < 2 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(h.lines(), vec!["blocker", "vol=70"]);
    }

    /// 退出收尾（P1-3）等的就是「线程真的退场了」这个事实：没关停时它不是退场，
    /// 排队中的作业没跑完时也不该算退场（命令线程关停后要先把手里的活干完）。
    #[test]
    fn wait_stopped_only_reports_true_after_the_worker_drained_and_left() {
        let h = Harness::new();
        assert!(!h.q.wait_stopped(Duration::from_millis(30)), "还没关停：谈不上退场");

        let gate = h.blocker();
        h.keyed("volume", "vol=70").expect("投递");
        h.q.shutdown();
        assert!(!h.q.wait_stopped(Duration::from_millis(50)), "排队中的作业还没跑完：不许算退场");

        gate.send(()).expect("放行");
        assert!(h.q.wait_stopped(Duration::from_secs(2)), "活干完、线程走人，才叫退场");
        assert_eq!(h.lines(), vec!["blocker", "vol=70"], "退场前要把排队中的作业跑完");
    }

    /// 空队列关停后立刻退场：退出路径上没播过歌（没起过 mpv）时不该白等。
    #[test]
    fn an_idle_worker_retires_right_after_shutdown() {
        let h = Harness::new();
        h.q.shutdown();
        assert!(h.q.wait_stopped(Duration::from_secs(2)));
    }
}
