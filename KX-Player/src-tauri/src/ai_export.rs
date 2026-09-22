//! AI-16 半自动模式：把「调模型」这一步整个换成用户自己跑。
//!
//! 为什么要它：`05` 第 16 条要求「不配置任何 Provider 时也有可用路径」（账本 AI-16）。前置
//! （读字幕 → 切段）和后置（装回原结构 → 写 `.zh.*`）本来就不依赖模型，中间那一步只要有一份
//! **能对齐**的文本就能替 —— 导出 `prompts.txt`，用户拿任何工具（更强的机器、别的软件、甚至
//! 人工）跑出译文，再导回来走同一套校验。文本长什么样在 `ai_wire.rs`。
//!
//! 序号口径：`04 §7.13` 写的是 `ai_job_items.seq`，而那张表在本项目里从未存在（AI 任务状态只在
//! 内存，见 `ai_job.rs` 的 `AI_FLAGS`）。所以这里用「本次选定的文件列表、按文件顺序、逐段」的
//! 1 起全局序号。它不需要持久化：导出与导入吃的是同一份 `paths`，**列表不变则序号不变**。
//!
//! 与 Provider 链路的一条**有意差异**：那边失败的块回退原文（宁可出混合文件也要出文件，见
//! `ai_chunk.rs`），这里整篇齐了才写盘。手工批次是可以补齐的 —— 把缺的序号再跑一遍、合并导入
//! 就行；写一个八成还是原文的 `.zh.srt` 只会让人以为已经翻完了。

use crate::ai_prompt as prompt;
use crate::ai_wire as wire;
use crate::subtitles as subs;
use std::collections::HashMap;

/// 差异预览里最多列几条样例（够看出翻得对不对，又不至于灌满前端）
const SAMPLE_LIMIT: usize = 6;

/// 一段待翻文本。只有 `seq` 是跨进程的约定（导入靠它对齐，不靠顺序），所以不存文件下标与
/// 行号 —— 那两个能从 `Entry.start` 推出来，多存一份就是多一处能对不上的地方。
pub struct Unit {
    pub seq: usize,
    pub text: String,
    /// false = 原文里含分隔线形状的行，本批不导出（导入时也就永远配不上）
    pub exportable: bool,
}

/// 一个成功解析的文件：占序号区间 `[start, start+count)`，外加装回去要用的原文档
pub struct Entry {
    pub path: String,
    kind: subs::Kind,
    start: usize,
    count: usize,
    doc: subs::Doc,
}

#[derive(Default)]
pub struct Loaded {
    pub units: Vec<Unit>,
    pub files: Vec<Entry>,
    /// 读不了 / 格式不认识 / 没有可翻译文本的文件，形状是 `"路径：原因"`
    pub errors: Vec<String>,
}

/// 读 + 解析整批路径。出问题的文件只进 `errors`、不占序号 —— 序号跳号无所谓，对齐靠的是序号。
pub fn load(paths: &[String]) -> Loaded {
    let mut out = Loaded::default();
    let mut docs: Vec<(String, subs::Doc)> = Vec::with_capacity(paths.len());
    for path in paths {
        match read_doc(path) {
            Ok(doc) => docs.push((path.clone(), doc)),
            Err(e) => out.errors.push(format!("{path}：{e}")),
        }
    }
    ingest(docs, &mut out);
    out
}

fn read_doc(path: &str) -> Result<subs::Doc, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("读取失败: {e}"))?;
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let doc = subs::parse(&content, &ext).ok_or_else(|| format!("不支持的字幕格式: .{ext}"))?;
    if doc.segment_count() == 0 {
        return Err("没有可翻译的文本".into());
    }
    Ok(doc)
}

/// 给已解析的文档按全局顺序发号。与磁盘分开，所以校验逻辑能纯内存测（本 crate 没有 tempfile）。
fn ingest(docs: Vec<(String, subs::Doc)>, out: &mut Loaded) {
    for (path, doc) in docs.into_iter() {
        let start = out.units.len() + 1;
        for (line, text) in doc.segments().iter().enumerate() {
            out.units.push(Unit { seq: start + line, text: text.clone(), exportable: wire::safe_to_export(text) });
        }
        let count = doc.segment_count();
        out.files.push(Entry { path, kind: doc.kind(), start, count, doc });
    }
}

/// 导出文本。`paths` 里一个能用的文件都没有时返回 None，调用方别去开保存框。
pub fn build_prompt(loaded: &Loaded) -> String {
    wire::build_prompt(
        loaded.units.iter().filter(|u| u.exportable).map(|u| (u.seq, u.text.as_str())),
    )
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    /// 实际写出的路径（保存框里用户定的）
    pub path: String,
    pub units: usize,
    pub exported: usize,
    /// 原文自带分隔线、本批导不了的条数
    pub unexportable: usize,
    pub files: usize,
    pub system_prompt: String,
    pub errors: Vec<String>,
}

/// 组导出结果。写盘由调用方负责，所以「写没写成」不会在这里被含糊掉。
pub fn export_meta(loaded: &Loaded, path: &str) -> ExportResult {
    let exported = loaded.units.iter().filter(|u| u.exportable).count();
    ExportResult {
        path: path.to_string(),
        units: loaded.units.len(),
        exported,
        unexportable: loaded.units.len() - exported,
        files: loaded.files.len(),
        system_prompt: prompt::SYSTEM_PROMPT.to_string(),
        errors: loaded.errors.clone(),
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileStat {
    pub path: String,
    pub name: String,
    pub total: usize,
    pub matched: usize,
    /// 每一条都配上译文了 —— 只有齐的文件会（在 `apply` 时）写盘
    pub complete: bool,
    /// 齐了才有的目标路径（没配齐 = 缺席）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out_path: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    pub seq: usize,
    pub file: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub units: usize,
    /// 导入文本里的块数
    pub blocks: usize,
    pub matched: usize,
    pub missing: usize,
    pub duplicates: usize,
    /// 序号不属于本批（多半是导出之后队列又重扫过、列表变了）
    pub out_of_range: usize,
    pub empty: usize,
    pub unexportable: usize,
    /// 齐了、可以写盘的文件数
    pub ready: usize,
    /// `apply = true` 时真正写出的路径
    pub written: Vec<String>,
    pub files: Vec<FileStat>,
    pub samples: Vec<Sample>,
    /// 读不了的文件 + 写盘失败
    pub errors: Vec<String>,
}

/// 校验并（`apply` 时）写盘。预览与落地是同一个函数，差别只在最后要不要碰磁盘 ——
/// 所以「预览说能写 3 篇」和「写盘写了 3 篇」不可能对不上。
pub fn import(loaded: &Loaded, text: &str, apply: bool) -> ImportResult {
    let blocks = wire::parse_blocks(text);
    let expected: HashMap<usize, ()> = loaded.units.iter().map(|u| (u.seq, ())).collect();
    let mut r = ImportResult {
        units: loaded.units.len(),
        blocks: blocks.total,
        duplicates: blocks.duplicates,
        empty: blocks.empty,
        out_of_range: blocks.map.keys().filter(|k| !expected.contains_key(k)).count(),
        unexportable: loaded.units.iter().filter(|u| !u.exportable).count(),
        errors: loaded.errors.clone(),
        ..Default::default()
    };
    for entry in &loaded.files {
        let mut translated: Vec<String> = Vec::with_capacity(entry.count);
        let mut matched = 0usize;
        for i in 0..entry.count {
            let unit = &loaded.units[(entry.start - 1) + i];
            let t = blocks.map.get(&unit.seq).map(|s| wire::fit(s, entry.kind)).unwrap_or_default();
            // 配上译文 = 有内容 **且** 这条原文本身没把切块打坏
            if !unit.exportable || t.is_empty() {
                translated.push(String::new()); // 空串 = `Doc::render` 保留原文
                continue;
            }
            matched += 1;
            if r.samples.len() < SAMPLE_LIMIT && t != unit.text {
                r.samples.push(Sample {
                    seq: unit.seq,
                    file: base_name(&entry.path),
                    from: unit.text.clone(),
                    to: t.clone(),
                });
            }
            translated.push(t);
        }
        r.matched += matched;
        let complete = matched == entry.count;
        let mut stat =
            FileStat { path: entry.path.clone(), name: base_name(&entry.path), total: entry.count, matched, complete, out_path: None };
        if complete {
            r.ready += 1;
            let out = prompt::translated_path(&entry.path, entry.kind.as_str());
            stat.out_path = Some(out.clone());
            if apply {
                match std::fs::write(&out, entry.doc.render(&translated)) {
                    Ok(_) => r.written.push(out),
                    Err(e) => r.errors.push(format!("{out}：写入失败 {e}")),
                }
            }
        }
        r.files.push(stat);
    }
    r.missing = r.units - r.matched;
    r
}

fn base_name(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn srt(cues: &[&str]) -> subs::Doc {
        let mut s = String::new();
        for (i, text) in cues.iter().enumerate() {
            s.push_str(&format!("{}\n00:00:{:02},000 --> 00:00:{:02},500\n{text}\n\n", i + 1, i + 1, i + 2));
        }
        subs::parse(&s, "srt").expect("测试数据必须是合法 srt")
    }

    fn lrc(lines: &[&str]) -> subs::Doc {
        let s: String = lines.iter().enumerate().map(|(i, t)| format!("[00:{:02}.00]{t}\n", i + 1)).collect();
        subs::parse(&s, "lrc").expect("测试数据必须是合法 lrc")
    }

    fn loaded_with(docs: Vec<(String, subs::Doc)>) -> Loaded {
        let mut l = Loaded::default();
        ingest(docs, &mut l);
        l
    }

    #[test]
    fn seq_numbers_across_files_without_gaps() {
        let l = loaded_with(vec![("/a/one.srt".into(), srt(&["甲", "乙"])), ("/a/two.srt".into(), srt(&["丙"]))]);
        let seqs: Vec<usize> = l.units.iter().map(|u| u.seq).collect();
        assert_eq!(seqs, vec![1, 2, 3]);
        assert_eq!((l.files[0].start, l.files[0].count, l.files[1].start), (1, 2, 3));
    }

    #[test]
    fn prompt_skips_the_lines_that_would_break_alignment() {
        let l = loaded_with(vec![("/a/one.srt".into(), srt(&["正常", "### 1 ###"]))]);
        assert_eq!(build_prompt(&l), "### 1 ###\n正常\n\n");
        let m = export_meta(&l, "/a/prompts.txt");
        assert_eq!((m.units, m.exported, m.unexportable, m.files), (2, 1, 1, 1));
        assert_eq!(m.system_prompt, prompt::SYSTEM_PROMPT, "提示词回传给前端复制，不进批次文件");
        // 它占号但不导出 → 导入永远配不上 → 整篇不算齐，不会被写坏
        let r = import(&l, "### 1 ###\n译文\n", true);
        assert_eq!((r.unexportable, r.matched, r.ready, r.written.len()), (1, 1, 0, 0));
    }

    #[test]
    fn import_aligns_by_index_not_by_order() {
        let l = loaded_with(vec![("/a/one.srt".into(), srt(&["甲", "乙", "丙"]))]);
        let r = import(&l, "### 3 ###\n丙文\n\n### 1 ###\n甲文\n\n### 2 ###\n乙文\n", false);
        assert_eq!((r.blocks, r.matched, r.missing, r.ready), (3, 3, 0, 1));
        assert_eq!(r.files[0].out_path.as_deref(), Some("/a/one.zh.srt"));
    }

    #[test]
    fn out_of_range_indexes_are_counted_and_ignored() {
        let l = loaded_with(vec![("/a/one.srt".into(), srt(&["甲"]))]);
        // 用户拿上一版（更多条）的批次跑的译文：多出来的不能塞进这一版
        let r = import(&l, "### 1 ###\n甲文\n### 9 ###\n别人的\n### 0 ###\n越界\n", false);
        assert_eq!((r.out_of_range, r.matched, r.ready), (2, 1, 1));
    }

    #[test]
    fn partial_batch_is_not_ready_but_says_which_file_is_short() {
        let l = loaded_with(vec![("/a/one.srt".into(), srt(&["甲", "乙"]))]);
        let r = import(&l, "### 1 ###\n甲文\n", true);
        assert_eq!((r.ready, r.missing, r.written.len()), (0, 1, 0), "缺一条就不算这篇的账");
        assert_eq!((r.files[0].matched, r.files[0].complete, r.files[0].out_path.as_deref()), (1, false, None));
    }

    /// 唯一碰磁盘的一条：`apply` 真写得出 `.zh.srt`，而预览一步连文件都不该创建。
    #[test]
    fn a_complete_batch_writes_the_translation_file() {
        let dir = std::env::temp_dir().join(format!("kx-ai16-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("song.srt");
        std::fs::write(&src, "1\n00:00:01,000 --> 00:00:02,500\n甲\n\n2\n00:00:02,000 --> 00:00:03,500\n乙\n").unwrap();
        let paths = vec![src.to_string_lossy().replace('\\', "/")];
        let l = load(&paths);
        assert!(l.errors.is_empty(), "{:?}", l.errors);
        assert_eq!(l.units.len(), 2);
        // 只配第一条：不落盘
        assert!(import(&l, "### 1 ###\n甲文\n", true).written.is_empty());
        let out = dir.join("song.zh.srt");
        let p = import(&l, "### 1 ###\n甲文\n\n### 2 ###\n乙文\n", false);
        assert_eq!((p.ready, p.written.len()), (1, 0));
        assert!(!out.exists(), "预览不该改动磁盘");
        assert_eq!(p.files[0].out_path.as_deref(), Some(out.to_string_lossy().replace('\\', "/").as_str()));
        let r = import(&l, "### 1 ###\n甲文\n\n### 2 ###\n乙文\n", true);
        assert_eq!(r.written, vec![out.to_string_lossy().replace('\\', "/")]);
        assert_eq!(r.errors, p.errors, "预览报没错误，写盘就不该冒出错误");
        let written = std::fs::read_to_string(&out).unwrap();
        assert!(written.contains("甲文") && written.contains("乙文"), "{written}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lrc_batches_come_back_as_one_line_per_timestamp() {
        let l = loaded_with(vec![("/a/one.lrc".into(), lrc(&["第一句", "第二句"]))]);
        let r = import(&l, "### 1 ###\n上\n阙\n### 2 ###\n下阙\n", false);
        assert_eq!((r.matched, r.ready), (2, 1));
        assert_eq!(r.samples[0].to, "上 阙", "换行必须在写盘前压平");
    }

    #[test]
    fn unreadable_files_report_and_keep_their_own_number_line() {
        let mut l = Loaded::default();
        l.errors.push("/a/broken.lrc：读取失败: 系统找不到指定的文件。".into());
        ingest(vec![("/a/ok.srt".into(), srt(&["甲"]))], &mut l);
        let r = import(&l, "### 1 ###\n甲文\n", false);
        assert_eq!((r.units, r.ready), (1, 1));
        assert_eq!((r.errors.len(), r.files.len()), (1, 1));
    }

    #[test]
    fn report_is_per_file_so_the_user_sees_which_batch_is_short() {
        let l = loaded_with(vec![("/a/one.srt".into(), srt(&["甲", "乙"])), ("/a/two.srt".into(), srt(&["丙"]))]);
        let r = import(&l, "### 1 ###\n甲文\n", false);
        assert_eq!((r.ready, r.missing), (0, 2));
        assert_eq!(r.files.iter().map(|f| (f.name.as_str(), f.matched, f.complete)).collect::<Vec<_>>(), vec![("one.srt", 1, false), ("two.srt", 0, false)]);
        // 补齐第二篇（序号 3）之后它才齐，第一篇依然卡在第 2 条
        let r2 = import(&l, "### 1 ###\n甲文\n### 3 ###\n丙文\n", false);
        assert_eq!(r2.files.iter().map(|f| f.complete).collect::<Vec<_>>(), vec![false, true]);
        assert_eq!((r2.ready, r2.files[1].out_path.as_deref()), (1, Some("/a/two.zh.srt")));
    }
}
