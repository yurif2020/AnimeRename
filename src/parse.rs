// SPDX-License-Identifier: GPL-3.0-only
// SPDX-FileCopyrightText: 2026 yurif2020
//! 动漫文件名 / 文件夹名解析
//!
//! 典型输入：`[MingY] Shuumatsu Touring [12][WebRip][JPSC].mkv`
//! 解析结果：
//!   group   = Some("MingY")
//!   title   = "Shuumatsu Touring"
//!   episode = Some("12")
//!   tags    = ["[WebRip]", "[JPSC]"]

use regex::Regex;
use std::sync::OnceLock;

/// 支持的配对括号（起 → 止）
const PAIRS: &[(char, char)] = &[
    ('[', ']'),
    ('(', ')'),
    ('{', '}'),
    ('\u{3010}', '\u{3011}'), // 【 】
    ('\u{300e}', '\u{300f}'), // 『 』
    ('\u{300c}', '\u{300d}'), // 「 」
    ('<', '>'),
];

/// 视频扩展名（小写，不含点）
pub const VIDEO_EXTS: &[&str] = &[
    "mkv", "mp4", "ts", "m2ts", "webm", "flv", "avi", "mov", "wmv", "rm", "rmvb", "mpg", "mpeg",
    "vob", "m4v", "3gp", "ogv", "divx", "asf", "strm",
];

/// 字幕/附属文件：跟随视频一起改名，但保留原始后缀
pub const SIDE_EXTS: &[&str] = &[
    "ass", "srt", "ssa", "vtt", "sub", "idx", "sup", "usf", "cdx", "nfo", "jpg", "jpeg", "png",
    "gif", "webp", "smi",
];

pub fn is_video(ext: &str) -> bool {
    VIDEO_EXTS.contains(&ext.to_ascii_lowercase().as_str())
}

pub fn is_sidecar(ext: &str) -> bool {
    SIDE_EXTS.contains(&ext.to_ascii_lowercase().as_str())
}

fn re_pure_ep() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"(?i)^\s*(?:ep\.?|episode|第|话|集)?\s*(\d{1,3})\s*(?:话|話|集|ep)?\s*(v\d+|version\s*\d+)?\s*$")
            .unwrap()
    })
}

fn re_s_e() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)\bS(\d{1,2})[\s._-]?E(?:P)?(\d{1,3})\b").unwrap())
}

fn re_cn_ep() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"第\s*(\d{1,3})\s*[话話集]").unwrap())
}

fn re_dash_ep() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    // regex crate 不支持先行断言，用「结尾或一个非数字字符」做右边界，
    // 这样 [2025] 这类四位数不会被当成集数。
    R.get_or_init(|| {
        Regex::new(r"(?:^|[\s~_])[-–—]\s*(\d{1,3})(\s*v\d+)?(?:\D|$)").unwrap()
    })
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    /// 括号外的普通文本
    Lit(String),
    /// 括号内容（不含括号）以及原始写法（含括号）
    Br(String, String),
}

fn tokenize(s: &str) -> Vec<Tok> {
    let mut out: Vec<Tok> = Vec::new();
    let mut lit = String::new();
    let mut chars = s.chars().peekable();
    // 当前打开的括号：(关闭符, 内容缓冲)
    let mut open: Option<(char, String)> = None;

    while let Some(c) = chars.next() {
        match &mut open {
            None => {
                if let Some((_, close)) = PAIRS.iter().find(|(o, _)| *o == c) {
                    if !lit.is_empty() {
                        out.push(Tok::Lit(std::mem::take(&mut lit)));
                    }
                    open = Some((*close, String::new()));
                } else {
                    lit.push(c);
                }
            }
            Some((close, buf)) => {
                if c == *close {
                    let content = std::mem::take(buf);
                    // 重建原始括号写法
                    let open_ch = PAIRS
                        .iter()
                        .find(|(_, cl)| *cl == *close)
                        .map(|(o, _)| *o)
                        .unwrap_or('[');
                    let orig: String = {
                        let mut t = String::new();
                        t.push(open_ch);
                        t.push_str(&content);
                        t.push(*close);
                        t
                    };
                    out.push(Tok::Br(content, orig));
                    open = None;
                } else if c == '\n' {
                    buf.push(' ');
                } else {
                    buf.push(c);
                }
            }
        }
    }
    // 未闭合的括号：把剩余内容当普通文本
    if let Some((_, mut buf)) = open {
        if !lit.is_empty() {
            out.push(Tok::Lit(std::mem::take(&mut lit)));
        }
        buf.insert(0, '[');
        out.push(Tok::Lit(buf));
    } else if !lit.is_empty() {
        out.push(Tok::Lit(lit));
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum BrKind {
    /// 集数，例如 [12] [第12话] [EP03] [12v2]
    Episode,
    /// 其它标签，例如 [WebRip] [JPSC] [MingY] [1080p]
    Tag,
}

fn classify(content: &str) -> BrKind {
    let c = content.trim();
    if c.is_empty() {
        return BrKind::Tag;
    }
    if let Some(cap) = re_pure_ep().captures(c) {
        let digits = &cap[1];
        let n: u32 = digits.parse().unwrap_or(9999);
        let has_marker = c.to_ascii_lowercase().contains("ep")
            || c.contains('第')
            || c.contains('话')
            || c.contains('集');
        // 数字部分 <=3 位且不超过 200 才算集数；4 位一般是年份。
        // 注意用数字长度而不是整串长度，否则 [12v2] 会被误判成标签。
        if has_marker || (digits.len() <= 3 && n <= 200) {
            return BrKind::Episode;
        }
        return BrKind::Tag;
    }
    if re_cn_ep().is_match(c) || re_s_e().is_match(c) {
        return BrKind::Episode;
    }
    BrKind::Tag
}

fn has_cjk(s: &str) -> bool {
    s.chars()
        .any(|c| ('\u{3040}'..='\u{30ff}').contains(&c) || ('\u{4e00}'..='\u{9fff}').contains(&c))
}

fn looks_like_group(content: &str) -> bool {
    let c = content.trim();
    if c.is_empty() || c.len() > 24 || has_cjk(c) {
        return false;
    }
    // 含数字的一般是分辨率/年份/集数，不像字幕组名
    !c.chars().any(|ch| ch.is_ascii_digit())
        && c.chars().any(|ch| ch.is_ascii_alphabetic())
        && !matches!(
            c.to_ascii_lowercase().as_str(),
            "ova" | "tv" | "bd" | "web" | "movie" | "sp" | "end" | "complete"
        )
}

fn norm_ep(num: &str, suffix: Option<&str>) -> String {
    let n: u32 = num.parse().unwrap_or(0);
    let mut s = if num.len() >= 3 || n >= 100 {
        num.to_string()
    } else if n < 10 {
        format!("0{n}")
    } else {
        format!("{n:02}")
    };
    if let Some(v) = suffix {
        let v = v.trim();
        if !v.is_empty() {
            s.push_str(v);
        }
    }
    s
}

/// 从字符串中提取集数；返回 (集数, 命中的字节偏移, 命中长度)
fn find_inline_ep(lit: &str) -> Option<(String, usize, usize)> {
    if let Some(cap) = re_s_e().captures(lit) {
        let (ep, s) = (&cap[2], &cap[1]);
        return Some((
            format!("S{s}E{}", norm_ep(ep, None)),
            cap.get(0).unwrap().start(),
            cap.get(0).unwrap().len(),
        ));
    }
    if let Some(cap) = re_cn_ep().captures(lit) {
        return Some((
            norm_ep(&cap[1], None),
            cap.get(0).unwrap().start(),
            cap.get(0).unwrap().len(),
        ));
    }
    if let Some(cap) = re_dash_ep().captures(lit) {
        let suffix = cap.get(2).map(|m| m.as_str());
        return Some((
            norm_ep(&cap[1], suffix),
            cap.get(0).unwrap().start(),
            cap.get(0).unwrap().len(),
        ));
    }
    None
}

#[derive(Debug, Clone, Default)]
pub struct Parsed {
    /// 清洗后的系列标题（罗马音/英文/日文原文）
    pub title: String,
    /// 规范化集数，例如 "12"、"03v2"、"S01E12"
    pub episode: Option<String>,
    /// 尾部标签，含原始括号，例如 ["[WebRip]", "[JPSC]"]
    pub tags: Vec<String>,
    /// 字幕组名，例如 "MingY"
    pub group: Option<String>,
    /// 标题之前的其它标签（不是字幕组的那些）
    pub lead_tags: Vec<String>,
}

/// 解析一个不含扩展名的名字（文件或文件夹）
pub fn parse_name(stem: &str) -> Parsed {
    let toks = tokenize(stem);

    // 第一遍：定位集数标记
    let mut ep_at: Option<(usize, Option<usize>, String)> = None; // (token idx, 字面量内偏移, 集数)
    for (i, t) in toks.iter().enumerate() {
        match t {
            Tok::Br(content, _) => {
                if classify(content) == BrKind::Episode {
                    if let Some(cap) = re_pure_ep().captures(content.trim()) {
                        let suffix = cap.get(2).map(|m| m.as_str());
                        let ep = norm_ep(&cap[1], suffix);
                        ep_at = Some((i, None, ep));
                    } else if let Some(cap) = re_s_e().captures(content) {
                        ep_at = Some((i, None, format!("S{}E{}", &cap[1], norm_ep(&cap[2], None))));
                    } else if let Some(cap) = re_cn_ep().captures(content) {
                        ep_at = Some((i, None, norm_ep(&cap[1], None)));
                    }
                    break;
                }
            }
            Tok::Lit(l) => {
                if let Some((ep, off, len)) = find_inline_ep(l) {
                    let _ = len;
                    ep_at = Some((i, Some(off), ep));
                    break;
                }
            }
        }
    }

    let ep_index = ep_at.as_ref().map(|(i, _, _)| *i);
    let mut title_buf = String::new();
    let mut tags: Vec<String> = Vec::new();
    let mut group: Option<String> = None;
    let mut lead_tags: Vec<String> = Vec::new();
    let mut seen_text = false; // 是否已进入标题区

    for (i, t) in toks.iter().enumerate() {
        if let Some(ei) = ep_index {
            if i > ei {
                // 集数之后的所有内容都归入尾部标签
                match t {
                    Tok::Br(_, orig) => tags.push(orig.clone()),
                    Tok::Lit(l) => {
                        let rest = clean_str(l);
                        if !rest.is_empty() {
                            tags.push(rest);
                        }
                    }
                }
                continue;
            }
        }
        match t {
            Tok::Br(content, orig) => {
                if i == ep_index.map(|x| x).unwrap_or(usize::MAX) {
                    continue; // 集数括号本身丢弃
                }
                if !seen_text {
                    if group.is_none() && looks_like_group(content) {
                        group = Some(content.trim().to_string());
                    } else {
                        lead_tags.push(orig.clone());
                    }
                } else {
                    tags.push(orig.clone());
                }
            }
            Tok::Lit(l) => {
                // 若集数标记位于这段文字内部，则只把它之前的部分算作标题
                let slice = match &ep_at {
                    Some((ti, Some(off), _)) if *ti == i => &l[..*off],
                    Some((ti, None, _)) if *ti == i => "",
                    _ => &l[..],
                };
                if !slice.is_empty() && slice.chars().any(|c| c.is_alphanumeric()) {
                    seen_text = true;
                }
                title_buf.push_str(slice);
            }
        }
    }

    let title = clean_title(&title_buf);
    Parsed {
        title,
        episode: ep_at.map(|(_, _, e)| e),
        tags,
        group,
        lead_tags,
    }
}

fn clean_str(s: &str) -> String {
    collapse_ws(s.trim_matches(|c: char| {
        c.is_whitespace() || "-~_|,".contains(c)
    }))
}

fn clean_title(s: &str) -> String {
    let mut t = s.replace('_', " ");
    t = collapse_ws(&t);
    // 去掉两端残留的分隔符
    let mut guard = 0;
    while guard < 20 {
        let before = t.len();
        t = t.trim_matches(|c: char| {
            c.is_whitespace() || "-–~|,.&/\\()[]{}【】「」『』:;!?*".contains(c)
        })
        .to_string();
        // 中间的 " - " 结尾也去掉
        t = t.trim_end_matches(|c: char| c.is_whitespace() || "-–~".contains(c)).to_string();
        guard += 1;
        if t.len() == before {
            break;
        }
    }
    collapse_ws(&t)
}

pub fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = false;
    for c in s.chars() {
        if c.is_whitespace() || c == '\u{3000}' {
            if !last_space && !out.is_empty() {
                out.push(' ');
            }
            last_space = true;
        } else {
            out.push(c);
            last_space = false;
        }
    }
    out
}

pub fn split_ext(name: &str) -> (String, String) {
    match name.rfind('.') {
        Some(i) if i > 0 => (name[..i].to_string(), name[i + 1..].to_ascii_lowercase()),
        _ => (name.to_string(), String::new()),
    }
}

/// Windows 非法字符清洗
pub fn sanitize_filename(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => out.push(' '),
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
    let mut out = collapse_ws(&out).trim().to_string();
    while out.ends_with('.') || out.ends_with(' ') {
        out.pop();
    }
    // 避免与 DOS 设备名冲突
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
        "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let stem_upper = out.split('.').next().unwrap_or("").to_ascii_uppercase();
    if RESERVED.contains(&stem_upper.as_str()) {
        out.push('_');
    }
    if out.chars().count() > 140 {
        out = out.chars().take(140).collect();
        out = out.trim().to_string();
    }
    if out.is_empty() {
        out = "未命名".to_string();
    }
    out
}

#[derive(Debug, Clone, Default)]
pub struct RenameOpts {
    /// 保留字幕组前缀，例如 "[MingY] "
    pub keep_group: bool,
    /// 保留标题前的其它标签
    pub keep_lead_tags: bool,
}

/// 生成新的文件名主干（不含扩展名）。`cn` 为中文标题。
pub fn build_stem(cn: &str, p: &Parsed, opts: &RenameOpts) -> String {
    let mut s = String::new();
    if opts.keep_group {
        if let Some(g) = &p.group {
            s.push_str(&format!("[{g}] "));
        }
    }
    if opts.keep_lead_tags {
        for t in &p.lead_tags {
            s.push_str(t);
        }
        if !p.lead_tags.is_empty() {
            s.push(' ');
        }
    }
    s.push_str(cn);
    if let Some(ep) = &p.episode {
        s.push_str(&format!(" - {ep}"));
    }
    if !p.tags.is_empty() {
        s.push(' ');
        s.push_str(&p.tags.join(""));
    }
    sanitize_filename(&collapse_ws(&s))
}

/// 生成新文件名（含扩展名）
pub fn build_file_name(cn: &str, p: &Parsed, ext: &str, opts: &RenameOpts) -> String {
    let mut name = build_stem(cn, p, opts);
    if !ext.is_empty() {
        name.push('.');
        name.push_str(ext);
    }
    name
}

/// 生成新文件夹名
pub fn build_folder_name(cn: &str, p: &Parsed, opts: &RenameOpts) -> String {
    build_file_name(cn, p, "", opts)
}

/// 多数投票：从一批文件里挑出出现次数最多的标题
pub fn majority_title(items: &[Parsed]) -> Option<String> {
    let mut counts: std::collections::HashMap<String, (usize, usize)> =
        std::collections::HashMap::new(); // title -> (count, first_seen)
    for (i, it) in items.iter().enumerate() {
        if it.title.chars().count() < 2 {
            continue;
        }
        let e = counts.entry(it.title.clone()).or_insert((0, i));
        e.0 += 1;
    }
    let mut v: Vec<(String, usize, usize)> = counts
        .into_iter()
        .map(|(t, (c, o))| (t, c, o))
        .collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.2.cmp(&b.2)));
    v.into_iter().next().map(|(t, _, _)| t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Parsed {
        parse_name(s)
    }

    #[test]
    fn ref_pattern() {
        let r = p("[MingY] Shuumatsu Touring [12][WebRip][JPSC]");
        assert_eq!(r.group.as_deref(), Some("MingY"));
        assert_eq!(r.title, "Shuumatsu Touring");
        assert_eq!(r.episode.as_deref(), Some("12"));
        assert_eq!(r.tags, vec!["[WebRip]", "[JPSC]"]);
    }

    #[test]
    fn dash_episode() {
        let r = p("[MingY] Shuumatsu Touring - 03 [WebRip][JPSC]");
        assert_eq!(r.title, "Shuumatsu Touring");
        assert_eq!(r.episode.as_deref(), Some("03"));
        assert_eq!(r.tags, vec!["[WebRip]", "[JPSC]"]);
    }

    #[test]
    fn se_and_cn_episode() {
        assert_eq!(
            p("[Sub] Show Name S01E05 [1080p]").episode.as_deref(),
            Some("S01E05")
        );
        assert_eq!(p("某动画 第12话 [GB]").episode.as_deref(), Some("12"));
    }

    #[test]
    fn year_is_not_episode() {
        let r = p("[MingY] Some Title [2025][WebRip]");
        assert_eq!(r.episode, None);
        assert_eq!(r.title, "Some Title");
        assert_eq!(r.tags, vec!["[2025]", "[WebRip]"]);
    }

    #[test]
    fn folder_name_has_no_episode() {
        let r = p("[MingY] Shuumatsu Touring [WebRip][JPSC]");
        assert_eq!(r.title, "Shuumatsu Touring");
        assert_eq!(r.episode, None);
        assert_eq!(r.tags, vec!["[WebRip]", "[JPSC]"]);
    }

    #[test]
    fn build_matches_target_format() {
        let r = p("[MingY] Shuumatsu Touring [12][WebRip][JPSC]");
        assert_eq!(
            build_file_name("末世二轮之旅", &r, "mkv", &RenameOpts::default()),
            "末世二轮之旅 - 12 [WebRip][JPSC].mkv"
        );
    }

    #[test]
    fn single_digit_pads() {
        let r = p("[MingY] Show [7][WebRip]");
        assert_eq!(r.episode.as_deref(), Some("07"));
    }

    #[test]
    fn version_suffix_kept() {
        let r = p("[MingY] Show [12v2][WebRip]");
        assert_eq!(r.episode.as_deref(), Some("12v2"));
    }

    #[test]
    fn cn_tags_and_brackets() {
        let r = p("【MingY】Shuumatsu Touring【12】【WebRip】");
        assert_eq!(r.episode.as_deref(), Some("12"));
        assert_eq!(r.title, "Shuumatsu Touring");
    }

    #[test]
    fn sanitize_removes_illegal() {
        assert_eq!(sanitize_filename("a<b>:c/d|e?f*g"), "a b c d e f g");
        assert_eq!(sanitize_filename("CON"), "CON_");
        assert_eq!(sanitize_filename("   "), "未命名");
    }
}
