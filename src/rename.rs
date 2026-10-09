//! 改名计划的构建、执行与撤销。
//! 只处理目标文件夹的第一层（不递归），符合「单层文件夹」的使用习惯。

use crate::parse::{self, Parsed, RenameOpts};
use crate::util;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Video,
    Sidecar,
}

#[derive(Debug, Clone)]
pub struct Row {
    pub old: String,
    pub new: String,
    pub ep: Option<String>,
    pub kind: Kind,
    pub note: String,
}

#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub dir: PathBuf,
    pub folder_old: String,
    pub folder_new: Option<String>,
    pub rows: Vec<Row>,
    /// 目录里含有视频的子目录数量，用于提醒用户本工具不递归
    pub subdirs_with_video: usize,
    pub skipped: Vec<String>,
}

impl Plan {
    pub fn video_rows(&self) -> impl Iterator<Item = &Row> {
        self.rows.iter().filter(|r| r.kind == Kind::Video)
    }
    pub fn change_count(&self) -> usize {
        self.rows.iter().filter(|r| r.old != r.new).count()
            + if self.folder_new.is_some() { 1 } else { 0 }
    }
}

/// 目标目录的安全检查
pub fn check_dir(dir: &Path) -> Result<(), String> {
    if !dir.exists() {
        return Err("路径不存在".to_string());
    }
    if !dir.is_dir() {
        return Err("所选路径不是文件夹".to_string());
    }
    if dir.parent().is_none() {
        return Err("不能对整个盘符操作".to_string());
    }
    // 盘符根目录（如 D:\）的 parent 是 Some("")
    if dir.parent().map(|p| p.as_os_str().is_empty()).unwrap_or(false) {
        return Err("不能对整个盘符操作".to_string());
    }
    let name = dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if name.is_empty() {
        return Err("无法识别该文件夹的名字".to_string());
    }
    Ok(())
}

/// 扫描目录并生成改名计划
pub fn build_plan(dir: &Path, cn: &str, opts: &RenameOpts) -> Result<Plan, String> {
    check_dir(dir)?;
    let cn = cn.trim();
    if cn.is_empty() {
        return Err("中文名为空，无法生成计划".to_string());
    }

    let mut plan = Plan {
        dir: dir.to_path_buf(),
        folder_old: dir
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default(),
        ..Default::default()
    };

    let mut entries: Vec<(String, String)> = Vec::new(); // (name, ext)
    let read = std::fs::read_dir(dir).map_err(|e| format!("读取目录失败：{e}"))?;
    for e in read.flatten() {
        let path = e.path();
        let fname = match path.file_name() {
            Some(f) => f.to_string_lossy().to_string(),
            None => continue,
        };
        if path.is_dir() {
            if has_video_in(&path) {
                plan.subdirs_with_video += 1;
            }
            continue;
        }
        let (stem, ext) = parse::split_ext(&fname);
        entries.push((stem, ext));
    }

    // 1) 视频文件
    let mut video_stems: HashMap<String, (Parsed, String)> = HashMap::new(); // old stem -> (parsed, new stem)
    let mut taken: HashSet<String> = HashSet::new(); // 已占用的新名（小写）
    let mut videos: Vec<(String, String, Parsed)> = Vec::new(); // (old stem, ext, parsed)
    for (stem, ext) in &entries {
        if parse::is_video(ext) {
            videos.push((stem.clone(), ext.clone(), parse::parse_name(stem)));
        }
    }
    videos.sort_by(|a, b| ep_sort_key(&a.2).cmp(&ep_sort_key(&b.2)));

    let mut used_new: HashSet<String> = HashSet::new();
    for (stem, ext, p) in &videos {
        let mut base = parse::build_stem(cn, p, opts);
        if used_new.contains(&base.to_lowercase()) {
            for n in 2..=99 {
                let cand = format!("{base} ({n})");
                if !used_new.contains(&cand.to_lowercase()) {
                    base = cand;
                    break;
                }
            }
        }
        used_new.insert(base.to_lowercase());
        let new_name = if ext.is_empty() {
            base.clone()
        } else {
            format!("{base}.{ext}")
        };
        let old_name = if ext.is_empty() {
            stem.clone()
        } else {
            format!("{stem}.{ext}")
        };
        let mut note = String::new();
        if p.episode.is_none() {
            note.push_str("未识别到集数；");
        }
        if taken.contains(&new_name.to_lowercase()) {
            note.push_str("与已有名称重复；");
        }
        taken.insert(new_name.to_lowercase());
        video_stems.insert(stem.clone(), (p.clone(), base.clone()));
        plan.rows.push(Row {
            old: old_name,
            new: new_name,
            ep: p.episode.clone(),
            kind: Kind::Video,
            note: note.trim_end_matches('；').to_string(),
        });
    }

    // 2) 字幕 / 封面等附属文件：跟着同名的视频一起改
    for (stem, ext) in entries.iter().filter(|(_, ext)| parse::is_sidecar(ext)) {
        if let Some((p, new_stem)) = video_stems.get(stem) {
            let old_name = format!("{stem}.{ext}");
            let new_name = format!("{new_stem}.{ext}");
            plan.rows.push(Row {
                old: old_name,
                new: new_name,
                ep: p.episode.clone(),
                kind: Kind::Sidecar,
                note: "跟随视频".to_string(),
            });
        } else {
            plan.skipped.push(format!("{stem}.{ext}"));
        }
    }

    // 3) 其它未知类型保持原样
    for (stem, ext) in entries.iter() {
        if !parse::is_video(ext) && !parse::is_sidecar(ext) {
            plan.skipped.push(if ext.is_empty() {
                stem.clone()
            } else {
                format!("{stem}.{ext}")
            });
        }
    }

    // 4) 文件夹本身
    let folder_parsed = parse::parse_name(&plan.folder_old);
    let mut merged = folder_parsed.clone();
    if merged.title.trim().is_empty() {
        // 文件夹名没带标题时，借用第一个视频文件的标签信息
        if let Some((_, _, first)) = videos.first() {
            merged.tags = first.tags.clone();
            merged.group = first.group.clone();
            merged.lead_tags = first.lead_tags.clone();
        }
    }
    merged.episode = None; // 文件夹不带集数
    let folder_new = parse::build_folder_name(cn, &merged, opts);
    plan.folder_new = if folder_new == plan.folder_old {
        None
    } else {
        Some(folder_new)
    };

    Ok(plan)
}

fn ep_sort_key(p: &Parsed) -> (u32, String) {
    match &p.episode {
        Some(e) => {
            let digits: String = e.chars().filter(|c| c.is_ascii_digit()).collect();
            (digits.parse::<u32>().unwrap_or(99_999), e.clone())
        }
        None => (u32::MAX, String::new()),
    }
}

fn has_video_in(dir: &Path) -> bool {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_file() {
                let (_, ext) = parse::split_ext(&p.to_string_lossy());
                if parse::is_video(&ext) {
                    return true;
                }
            }
        }
    }
    false
}

// ---------------------------------------------------------------- 执行

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Op {
    pub old: String,
    pub new: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Journal {
    pub ts: u64,
    pub dir: String,
    pub ops: Vec<Op>,
}

pub fn journal_path() -> PathBuf {
    util::ensure_app_data();
    util::app_data_dir().join("last_rename.json")
}

pub fn load_journal() -> Option<Journal> {
    let p = journal_path();
    std::fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str::<Journal>(&s).ok())
}

pub fn clear_journal() {
    let p = journal_path();
    if p.exists() {
        // 保留可追溯性：不删除，改名归档
        let used = util::app_data_dir().join("last_rename.used.json");
        let _ = std::fs::remove_file(&used);
        let _ = std::fs::rename(&p, used);
    }
}

/// 执行改名。返回逐条日志。任何一条失败都会记录下来，但不会中断其余条目。
pub fn apply(plan: &Plan, rename_folder: bool) -> Vec<String> {
    let mut log = Vec::new();
    let mut ops: Vec<Op> = Vec::new();

    for row in &plan.rows {
        if row.old == row.new {
            continue;
        }
        let old = plan.dir.join(&row.old);
        let new = plan.dir.join(&row.new);
        if !old.exists() {
            log.push(format!("跳过：{} 已不存在", row.old));
            continue;
        }
        if new.exists() {
            log.push(format!("跳过：目标已存在 “{}”", row.new));
            continue;
        }
        match std::fs::rename(&old, &new) {
            Ok(_) => {
                ops.push(Op {
                    old: old.to_string_lossy().to_string(),
                    new: new.to_string_lossy().to_string(),
                });
                log.push(format!("✔ {} → {}", row.old, row.new));
            }
            Err(e) => log.push(format!("✘ {} 失败：{e}", row.old)),
        }
    }

    if rename_folder {
        if let Some(new_folder) = &plan.folder_new {
            let old = plan.dir.clone();
            let parent = match old.parent() {
                Some(p) => p.to_path_buf(),
                None => {
                    log.push("✘ 无法取得上级目录，跳过文件夹改名".to_string());
                    return log;
                }
            };
            let new = parent.join(new_folder);
            if new.exists() {
                log.push(format!("✘ 文件夹目标已存在：{new_folder}"));
            } else {
                match std::fs::rename(&old, &new) {
                    Ok(_) => {
                        ops.push(Op {
                            old: old.to_string_lossy().to_string(),
                            new: new.to_string_lossy().to_string(),
                        });
                        log.push(format!("✔ 文件夹 → {new_folder}"));
                    }
                    Err(e) => log.push(format!("✘ 文件夹改名失败：{e}（文件可能正被播放器占用）")),
                }
            }
        } else {
            log.push("文件夹名已符合目标，无需修改".to_string());
        }
    }

    if !ops.is_empty() {
        let j = Journal {
            ts: util::now_ts(),
            dir: plan.dir.to_string_lossy().to_string(),
            ops,
        };
        if let Ok(s) = serde_json::to_string_pretty(&j) {
            let _ = std::fs::write(journal_path(), s);
            log.push("已写入撤销记录，可在界面点「撤销上次改名」恢复。".to_string());
        }
    }
    log
}

/// 撤销：按相反顺序还原
pub fn undo(j: &Journal) -> Vec<String> {
    let mut log = Vec::new();
    let mut done = 0usize;
    for op in j.ops.iter().rev() {
        let old = PathBuf::from(&op.old);
        let new = PathBuf::from(&op.new);
        if !new.exists() {
            log.push(format!("跳过：{} 不存在", op.new));
            continue;
        }
        if old.exists() {
            log.push(format!("跳过：原名 {} 已被占用", op.old));
            continue;
        }
        match std::fs::rename(&new, &old) {
            Ok(_) => {
                done += 1;
                log.push(format!("↩ {} → {}", op.new, op.old));
            }
            Err(e) => log.push(format!("✘ 还原失败 {}：{e}", op.new)),
        }
    }
    log.push(format!("共还原 {done}/{} 项", j.ops.len()));
    if done == j.ops.len() {
        clear_journal();
        log.push("撤销记录已归档。".to_string());
    }
    log
}
