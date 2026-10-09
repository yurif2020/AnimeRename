//! Bangumi (bgm.tv) 查询与候选打分。
//!
//! 接口：`POST https://api.bgm.tv/v0/search/subjects`（免鉴权、真实按 keyword 过滤）
//! 说明：搜索返回的顺序本身已经带有相关性，而查询词常是罗马音、命中的却是日文名，
//! 字面相似度会接近 0。因此打分只做「小幅加、明显减」，并保留返回顺序作为先验。

use crate::util::app_data_dir;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use unicode_normalization::UnicodeNormalization;

pub const API_SEARCH: &str = "https://api.bgm.tv/v0/search/subjects";
pub const UA: &str = "AnimeRename/1.0 (desktop file renamer; +https://bgm.tv)";
/// 条目类型：2 = 动画
pub const TYPE_ANIME: u8 = 2;

#[derive(Debug, Clone)]
pub struct Cand {
    pub id: u64,
    pub name: String,
    pub name_cn: String,
    pub date: String,
    pub rating: f64,
    pub score: f64,
}

impl Cand {
    /// 下拉框里的展示文本
    pub fn display(&self) -> String {
        let cn = if self.name_cn.trim().is_empty() {
            "（无中文名）"
        } else {
            self.name_cn.as_str()
        };
        let year = self
            .date
            .split('-')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("????");
        let rate = if self.rating > 0.0 {
            format!(", ★{:.1}", self.rating)
        } else {
            String::new()
        };
        format!("{cn} — {} ({year}{rate})", self.name)
    }

    /// 该候选是否适合直接用作文件名
    pub fn best_cn(&self) -> Option<String> {
        let t = self.name_cn.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Outcome {
    pub cands: Vec<Cand>,
    /// 实际生效的网络路径描述
    pub via: String,
    pub cached: bool,
    pub error: Option<String>,
}

// ---------------------------------------------------------------- 网络层

fn agent_for(proxy: Option<&str>) -> ureq::Agent {
    let mut b = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(12))
        .user_agent(UA);
    // ureq 里 native-tls 不会自动生效，必须显式挂到 agent 上。
    // 走系统 schannel：不需要 C 工具链，并且使用 Windows 证书库。
    // （本项目固定以 native-tls 特性构建 ureq，故无需 cfg 判断）
    if let Ok(tc) = ureq::native_tls::TlsConnector::new() {
        b = b.tls_connector(std::sync::Arc::new(tc));
    }
    if let Some(p) = proxy {
        if let Ok(pr) = ureq::Proxy::new(p) {
            b = b.proxy(pr);
        }
    }
    b.build()
}

/// 依次尝试：系统代理（两次）→ 直连（两次）。
/// 本机实测首次 CONNECT 偶发失败，重试即可恢复。
fn post_search(keyword: &str, proxy: Option<&str>) -> Result<(Vec<Cand>, String), String> {
    let mut routes: Vec<(Option<String>, &str)> = Vec::new();
    if let Some(p) = proxy {
        routes.push((Some(p.to_string()), "系统代理"));
        routes.push((Some(p.to_string()), "系统代理(重试)"));
    }
    routes.push((None, "直连"));
    routes.push((None, "直连(重试)"));

    let body = serde_json::json!({
        "keyword": keyword,
        "type": TYPE_ANIME,
        "max_results": 25,
    });

    let mut last = String::from("未发起请求");
    for (i, (p, label)) in routes.iter().enumerate() {
        if i > 0 {
            std::thread::sleep(Duration::from_millis(250 * i as u64));
        }
        let agent = agent_for(p.as_deref());
        let resp = agent.post(API_SEARCH).send_json(body.clone());
        match resp {
            Ok(r) => {
                let status = r.status();
                let txt = r
                    .into_string()
                    .map_err(|e| format!("读取响应失败：{e}"))?;
                if status != 200 {
                    last = format!("{label} 返回 HTTP {status}");
                    continue;
                }
                let raw: serde_json::Value = match serde_json::from_str(&txt) {
                    Ok(v) => v,
                    Err(e) => {
                        last = format!("{label} JSON 解析失败：{e}");
                        continue;
                    }
                };
                let cands = parse_items(&raw, keyword);
                return Ok((cands, label.to_string()));
            }
            Err(e) => {
                last = format!("{label} 请求失败：{}", short_err(&e));
            }
        }
    }
    Err(last)
}

fn short_err(e: &ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, _) => format!("HTTP {code}"),
        other => other.to_string(),
    }
}

fn parse_items(v: &serde_json::Value, keyword: &str) -> Vec<Cand> {
    let arr = if let Some(a) = v.get("data").and_then(|d| d.as_array()) {
        a.clone()
    } else if let Some(a) = v.get("subjects").and_then(|d| d.as_array()) {
        a.clone()
    } else if let Some(a) = v.as_array() {
        a.clone()
    } else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for (pos, it) in arr.iter().enumerate() {
        // 兼容 {id,name,...} 与 {score, type, subject:{...}} 两种包装
        let e = it.get("subject").unwrap_or(it);
        let sarr = |k: &str| -> String {
            e.get(k)
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string()
        };
        let id = e.get("id").and_then(|x| x.as_u64()).unwrap_or(0);
        let name = sarr("name");
        let name_cn = sarr("name_cn");
        let date = sarr("date");
        let stype = e.get("type").and_then(|x| x.as_u64()).unwrap_or(0) as u8;
        let rating = e
            .get("score")
            .and_then(|x| x.as_f64())
            .unwrap_or(0.0);
        let score = score_candidate(keyword, pos, id, &name, &name_cn, stype, rating);
        out.push(Cand {
            id,
            name,
            name_cn,
            date,
            rating,
            score,
        });
    }
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

// ---------------------------------------------------------------- 打分

fn is_music(s: &str) -> bool {
    let l = s.to_lowercase();
    const HITS: &[&str] = &[
        "ost",
        "original soundtrack",
        "soundtrack",
        "character song",
        "insert song",
        "single",
        "album",
        "cd",
        "主题曲",
        "片头",
        "片尾",
        "插曲",
        "角色歌",
        "音乐",
        "原声",
        "キャラソン",
        "テーマ",
    ];
    HITS.iter().any(|h| l.contains(h))
}

fn is_sequel(s: &str) -> bool {
    let l = s.to_lowercase();
    const HITS: &[&str] = &[
        "第2期", "第3期", "第4期", "二期", "三期", "第二季", "第三季", "season 2", "season 3",
        "part 2", "part 3", "ii", "iii", "续集", "续篇", "2nd", "3rd",
    ];
    HITS.iter().any(|h| l.contains(h))
}

fn rank_prior(pos: usize) -> f64 {
    match pos {
        0 => 1.0,
        1 => 0.62,
        2 => 0.45,
        3 => 0.34,
        4 => 0.26,
        _ => 0.16,
    }
}

fn score_candidate(
    keyword: &str,
    pos: usize,
    _id: u64,
    name: &str,
    name_cn: &str,
    stype: u8,
    rating: f64,
) -> f64 {
    let q = flat(keyword);
    let mut sim = 0.0f64;
    for (txt, w) in [(flat(name), 1.0), (flat(name_cn), 0.97)] {
        if txt.is_empty() || q.is_empty() {
            continue;
        }
        let s = if txt == q {
            1.0
        } else if txt.starts_with(&q) || q.starts_with(&txt) {
            0.9
        } else if txt.contains(&q) || q.contains(&txt) {
            0.82
        } else {
            let d = dice(&txt, &q);
            let t = token_sim(&words(name).join(" "), &words(keyword).join(" "));
            d.max(t * 0.96)
        };
        sim = sim.max(s * w);
    }

    let mut sc = 0.30 + 0.60 * sim + 0.15 * rank_prior(pos);
    if stype != TYPE_ANIME {
        sc -= 0.50;
    }
    if is_music(name) || is_music(name_cn) {
        sc -= 0.45;
    }
    if name_cn.trim().is_empty() {
        sc -= 0.20;
    }
    if is_sequel(name) && is_sequel(keyword) == false {
        sc -= 0.10;
    }
    if rating > 0.0 {
        sc += (rating.min(10.0) / 10.0) * 0.06;
    }
    sc
}

fn flat(s: &str) -> String {
    let t: String = s.nfkc().collect();
    t.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

fn words(s: &str) -> Vec<String> {
    let t: String = s.nfkc().collect();
    t.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_string())
        .collect()
}

/// 字符二元组 Dice 系数，对罗马音/英文的拼写差异更宽容
fn dice(a: &str, b: &str) -> f64 {
    let ca: Vec<char> = a.chars().collect();
    let cb: Vec<char> = b.chars().collect();
    if ca.is_empty() || cb.is_empty() {
        return 0.0;
    }
    if ca.len() == 1 || cb.len() == 1 {
        return if ca == cb { 1.0 } else { 0.0 };
    }
    let mut sa: HashMap<(char, char), u32> = HashMap::new();
    for w in ca.windows(2) {
        *sa.entry((w[0], w[1])).or_insert(0) += 1;
    }
    let mut sb: HashMap<(char, char), u32> = HashMap::new();
    for w in cb.windows(2) {
        *sb.entry((w[0], w[1])).or_insert(0) += 1;
    }
    let inter: u32 = sa.iter().map(|(k, v)| (*v).min(sb.get(k).copied().unwrap_or(0))).sum();
    let total: u32 = sa.values().sum::<u32>() + sb.values().sum::<u32>();
    2.0 * inter as f64 / total as f64
}

fn token_sim(a: &str, b: &str) -> f64 {
    let wa: std::collections::HashSet<String> = words(a).into_iter().collect();
    let wb: std::collections::HashSet<String> = words(b).into_iter().collect();
    if wa.is_empty() || wb.is_empty() {
        return 0.0;
    }
    let inter = wa.intersection(&wb).count() as f64;
    let union = wa.union(&wb).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

// ---------------------------------------------------------------- 缓存

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hit {
    pub id: u64,
    pub name: String,
    pub name_cn: String,
    #[serde(default)]
    pub date: String,
    pub ts: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Cache {
    #[serde(default)]
    map: HashMap<String, Hit>,
}

fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn cache_file() -> PathBuf {
    app_data_dir().join("cache.json")
}

impl Cache {
    pub fn load() -> Cache {
        let path = cache_file();
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let path = cache_file();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(&path, s);
        }
    }

    pub fn get(&self, keyword: &str) -> Option<&Hit> {
        self.map.get(&flat(keyword))
    }

    pub fn put(&mut self, keyword: &str, c: &Cand) {
        if c.name_cn.trim().is_empty() {
            return;
        }
        self.map.insert(
            flat(keyword),
            Hit {
                id: c.id,
                name: c.name.clone(),
                name_cn: c.name_cn.clone(),
                date: c.date.clone(),
                ts: now_ts(),
            },
        );
    }
}

/// 对外主入口：缓存优先，其次在线查询
pub fn lookup(keyword: &str, proxy: Option<&str>, cache: &mut Cache, use_cache: bool) -> Outcome {
    if use_cache {
        if let Some(h) = cache.get(keyword).cloned() {
            return Outcome {
                cands: vec![Cand {
                    id: h.id,
                    name: h.name,
                    name_cn: h.name_cn,
                    date: h.date,
                    rating: 0.0,
                    score: 1.0,
                }],
                via: "本地缓存".to_string(),
                cached: true,
                error: None,
            };
        }
    }

    match post_search(keyword, proxy) {
        Ok((mut cands, via)) => {
            if cands.is_empty() {
                return Outcome {
                    cands,
                    via,
                    cached: false,
                    error: Some("Bangumi 没有返回任何条目，可换个关键词或手动填写中文名".to_string()),
                };
            }
            // 最优候选缺中文名时，补拉详情接口再试一次
            if cands[0].name_cn.trim().is_empty() {
                let id = cands[0].id;
                if let Some(cn) = fetch_name_cn(id, proxy.as_deref()) {
                    cands[0].name_cn = cn;
                }
            }
            if let Some(best) = cands.iter().find(|c| !c.name_cn.trim().is_empty()) {
                cache.put(keyword, best);
                cache.save();
            }
            Outcome {
                cands,
                via,
                cached: false,
                error: None,
            }
        }
        Err(e) => Outcome {
            cands: Vec::new(),
            via: String::new(),
            cached: false,
            error: Some(e),
        },
    }
}

fn fetch_name_cn(id: u64, proxy: Option<&str>) -> Option<String> {
    if id == 0 {
        return None;
    }
    let url = format!("https://api.bgm.tv/v0/subjects/{id}");
    let mut routes: Vec<Option<String>> = Vec::new();
    if let Some(p) = proxy {
        routes.push(Some(p.to_string()));
    }
    routes.push(None);
    for p in routes {
        let agent = agent_for(p.as_deref());
        if let Ok(r) = agent.get(&url).call() {
            if let Ok(v) = r.into_json::<serde_json::Value>() {
                let cn = v.get("name_cn").and_then(|x| x.as_str()).unwrap_or("");
                if !cn.trim().is_empty() {
                    return Some(cn.trim().to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn music_is_penalised() {
        let ost = score_candidate("Frieren", 0, 1, "TVアニメ『葬送のフリーレン』Original Soundtrack", "", 2, 8.0);
        let tv = score_candidate("Frieren", 1, 2, "葬送のフリーレン", "葬送的芙莉莲", 2, 9.0);
        assert!(tv > ost, "OST 应排在正片之后：ost={ost} tv={tv}");
    }

    #[test]
    fn non_anime_penalised() {
        let a = score_candidate("Frieren", 0, 1, "葬送のフリーレン", "葬送的芙莉莲", 1, 9.0);
        let b = score_candidate("Frieren", 1, 2, "葬送のフリーレン", "葬送的芙莉莲", 2, 9.0);
        assert!(b > a);
    }

    #[test]
    fn ranking_keeps_bgm_order_when_script_differs() {
        // 罗马音查询 vs 日文条目名，字面相似度接近 0，靠位置先验保证首条胜出
        let first = score_candidate("Shuumatsu Touring", 0, 1, "終末ツーリング", "末世二轮之旅", 2, 7.0);
        let later = score_candidate("Shuumatsu Touring", 5, 2, "少女終末旅行", "少女终末旅行", 2, 8.5);
        assert!(first > later, "first={first} later={later}");
    }

    #[test]
    fn dice_basics() {
        assert!(dice("frieren", "frieren") > 0.99);
        assert!(dice("touring", "turing") > 0.5);
        assert!(dice("abcd", "wxyz") < 0.05);
    }
}
