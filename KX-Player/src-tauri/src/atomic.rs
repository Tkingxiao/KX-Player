//! 原子落盘（P0-1 后半）：唯一临时名 → 写 → flush → sync → 同目录 rename 替换。
//!
//! 为什么不直接 `fs::write`：这些文件（设置、改名账本、封面、译文、背景图）都是下一次
//! 启动还要读回来的东西。写到一半崩溃/断电会留下一份半截 JSON 或半截 JPEG，用户看到的
//! 是「设置全丢了 / 封面坏了 / 译文缺了一半」，而且没有任何恢复线索 —— 原文件本来可能是好的。
//!
//! 两条硬要求，都踩过：
//! - **临时名必须唯一**（pid + 进程内计数 + 纳秒）。固定名（`x.json.tmp`）在同一目录里
//!   两个写入者会互相踩：一个刚写完还没 rename，另一个把临时件覆盖了，rename 上去的是
//!   另一份内容，而且两边都报成功。
//! - **临时件必须和目标同目录**。跨卷 rename 不是原子替换，`fs::rename` 在 Windows 上
//!   直接失败；同目录内 rename 才是「要么旧的、要么新的」。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

/// 目标旁边的唯一临时名。前导点让它在目录列表里显眼（Windows 不隐藏，但排在最前）。
fn temp_path(target: &Path) -> PathBuf {
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "tmp".into());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    target.with_file_name(format!(".{name}.{}.{}.{}.tmp", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed), nanos))
}

/// 原子替换 `target` 的内容。父目录不存在时一并建出来（封面目录、数据目录首次写入）。
pub fn write(target: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = temp_path(target);
    let out = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        // flush 把 Rust 自己的缓冲交给内核，sync_all 再让内核把数据落到盘上 ——
        // 少了 sync，rename 之后断电仍可能是「新名字指向旧内容」。
        f.flush()?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, target)
    })();
    if out.is_err() {
        // 失败时不留临时件：留着既占空间，也会让下一次的「目录里有没有 .tmp」判断失真
        let _ = std::fs::remove_file(&tmp);
    }
    out
}

pub fn write_str(target: &Path, text: &str) -> std::io::Result<()> {
    write(target, text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个用例一个独立目录，Drop 时清掉（与 scope.rs 的夹具同口径）。
    struct Dir(PathBuf);

    impl Dir {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let p = std::env::temp_dir().join(format!("kx-atomic-{tag}-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&p).expect("建测试目录");
            Self(p)
        }
        fn file(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .expect("读目录")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect()
    }

    #[test]
    fn writes_a_new_file_and_leaves_no_temp() {
        let d = Dir::new("new");
        let target = d.file("x.json");
        write_str(&target, "{\"a\":1}").expect("写成功");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "{\"a\":1}");
        assert!(leftovers(&d.0).is_empty(), "成功后不该留临时件：{:?}", leftovers(&d.0));
    }

    #[test]
    fn replaces_existing_content_in_one_step() {
        let d = Dir::new("replace");
        let target = d.file("x.json");
        std::fs::write(&target, "旧内容").unwrap();
        write_str(&target, "新内容").expect("替换成功");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "新内容");
        assert!(leftovers(&d.0).is_empty());
    }

    #[test]
    fn creates_missing_parent_directories() {
        let d = Dir::new("parents");
        let target = d.file("covers").join("deep").join("a.jpg");
        write(&target, b"jpeg").expect("写成功");
        assert_eq!(std::fs::read(&target).unwrap(), b"jpeg");
    }

    /// 两个线程抢同一个目标：最终内容必须**整份**是其中一份，不能是你一半我一半。
    /// 固定临时名（`x.tmp`）在这里就会露馅 —— 两边写同一个临时件，rename 上去的内容
    /// 谁也说不清，而且两边都返回成功。
    #[test]
    fn concurrent_writers_never_interleave_content() {
        let d = Dir::new("race");
        let target = d.file("map.json");
        let a = vec![b'a'; 512 * 1024];
        let b = vec![b'b'; 512 * 1024];
        let mut handles = Vec::new();
        for payload in [a.clone(), b.clone(), a.clone(), b.clone()] {
            let t = target.clone();
            handles.push(std::thread::spawn(move || {
                for _ in 0..8 {
                    write(&t, &payload).expect("写成功");
                }
            }));
        }
        for h in handles {
            h.join().expect("线程不 panic");
        }
        let got = std::fs::read(&target).expect("读过最终内容");
        assert!(got == a || got == b, "最终内容必须是某一份的整份，实测 {} 字节", got.len());
        assert!(leftovers(&d.0).is_empty(), "争用结束后不该留临时件：{:?}", leftovers(&d.0));
    }

    /// 临时件必须建在目标同目录：跨卷 rename 不是原子替换，Windows 上直接失败。
    /// 这里用「目标在子目录、临时件也在子目录」来钉住实现（父目录建出来后临时件跟着走）。
    #[test]
    fn temp_file_sits_next_to_the_target() {
        let d = Dir::new("samevol");
        let target = d.file("sub").join("a.json");
        let tmp = temp_path(&target);
        assert_eq!(tmp.parent(), target.parent(), "临时件必须与目标同目录");
        assert_ne!(tmp, target);
        write_str(&target, "x").expect("写成功");
        assert!(target.exists());
    }

    #[test]
    fn a_failed_write_reports_and_cleans_up() {
        let d = Dir::new("fail");
        // 父路径是一个文件 → create_dir_all 必然失败，不该留下任何东西
        let blocker = d.file("blocker");
        std::fs::write(&blocker, "x").unwrap();
        let target = blocker.join("a.json");
        assert!(write_str(&target, "y").is_err());
        assert!(leftovers(&d.0).is_empty(), "失败路径也要清临时件：{:?}", leftovers(&d.0));
    }
}
