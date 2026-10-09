// SPDX-License-Identifier: GPL-3.0-only
// SPDX-FileCopyrightText: 2026 yurif2020
//! 通用路径与时间工具

use std::path::PathBuf;

/// 程序数据目录：%LOCALAPPDATA%\AnimeRename（查询缓存、撤销记录）
pub fn app_data_dir() -> PathBuf {
    for key in ["LOCALAPPDATA", "AppData"] {
        if let Ok(v) = std::env::var(key) {
            if !v.trim().is_empty() {
                return PathBuf::from(v).join("AnimeRename");
            }
        }
    }
    if let Ok(v) = std::env::var("HOME") {
        return PathBuf::from(v).join(".anime_rename");
    }
    std::env::temp_dir().join("AnimeRename")
}

pub fn ensure_app_data() -> PathBuf {
    let dir = app_data_dir();
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 本地时间字符串，用于日志与撤销记录
pub fn fmt_ts(ts: u64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(ts as i64, 0)
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| format!("{ts}"))
}

