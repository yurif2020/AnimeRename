#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! 动漫文件名 / 文件夹中文重命名
//!
//! 默认启动图形界面；带 `--cli` 参数时以命令行方式运行，便于脚本化与自检。

mod app;
mod dpi;
mod bgm;
mod parse;
mod proxy;
mod rename;
mod util;

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{}", HELP);
        return;
    }
    if args.first().map(|a| a.as_str()) == Some("--cli") {
        if args.iter().any(|a| a == "--undo") {
            match rename::load_journal() {
                Some(j) => {
                    println!("撤销 {} 项操作（{}）", j.ops.len(), j.dir);
                    for l in rename::undo(&j) {
                        println!("{l}");
                    }
                }
                None => println!("没有可撤销的记录。"),
            }
            return;
        }
        if let Err(e) = cli(&args[1..]) {
            eprintln!("错误：{e}");
            std::process::exit(1);
        }
        return;
    }
    let dir = args
        .first()
        .map(PathBuf::from)
        .filter(|p| p.is_dir());
    if let Err(e) = app::run(dir) {
        eprintln!("界面启动失败：{e}");
        std::process::exit(1);
    }
}

const HELP: &str = "\
动漫文件名中文重命名

图形界面（默认）：
  AnimeRename.exe [起始文件夹]

命令行：
  AnimeRename.exe --cli <文件夹> [选项]
    --undo            撤销最近一次由本程序执行的改名（可与 --cli 连用）
    --cn <中文名>     跳过在线查询，直接指定中文片名
    --kw <查询词>     覆盖自动推断的系列原名
    --apply           真正执行改名（默认只预览）
    --proxy <地址>    指定代理，例如 http://127.0.0.1:7890
    --direct          强制直连，忽略系统代理
    --no-cache        忽略本地查询缓存
    --keep-group      保留字幕组前缀，例如 [MingY]
    --no-folder       不改文件夹名
    --json            以 JSON 输出计划

说明：只处理该文件夹第一层的视频与字幕文件，不递归子目录。
";

struct CliOpts {
    dir: PathBuf,
    cn: Option<String>,
    kw: Option<String>,
    apply: bool,
    proxy: Option<String>,
    direct: bool,
    no_cache: bool,
    opts: parse::RenameOpts,
    rename_folder: bool,
    json: bool,
}

fn cli(args: &[String]) -> Result<(), String> {
    let mut o = CliOpts {
        dir: PathBuf::new(),
        cn: None,
        kw: None,
        apply: false,
        proxy: None,
        direct: false,
        no_cache: false,
        opts: parse::RenameOpts::default(),
        rename_folder: true,
        json: false,
    };
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let next = |i: &mut usize| -> Result<String, String> {
            *i += 1;
            args.get(*i)
                .cloned()
                .ok_or_else(|| format!("{a} 需要一个参数"))
        };
        match a {
            "--cn" => o.cn = Some(next(&mut i)?),
            "--kw" => o.kw = Some(next(&mut i)?),
            "--proxy" => o.proxy = Some(next(&mut i)?),
            "--apply" => o.apply = true,
            "--direct" => o.direct = true,
            "--no-cache" => o.no_cache = true,
            "--keep-group" => o.opts.keep_group = true,
            "--no-folder" => o.rename_folder = false,
            "--json" => o.json = true,
            s if s.starts_with('-') => return Err(format!("未知参数 {s}")),
            s => {
                if o.dir.as_os_str().is_empty() {
                    o.dir = PathBuf::from(s);
                } else {
                    return Err("只能指定一个文件夹".to_string());
                }
            }
        }
        i += 1;
    }
    if o.dir.as_os_str().is_empty() {
        return Err("请用 --cli <文件夹> 指定目标目录".to_string());
    }
    let dir = if o.dir.is_absolute() {
        o.dir.clone()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(&o.dir)
    };
    rename::check_dir(&dir)?;

    // 推断查询词
    let folder_p = parse::parse_name(
        &dir.file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default(),
    );
    let mut items = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_file() {
                continue;
            }
            let (stem, ext) = parse::split_ext(
                &p.file_name().unwrap_or_default().to_string_lossy(),
            );
            if parse::is_video(&ext) {
                items.push(parse::parse_name(&stem));
            }
        }
    }
    if items.is_empty() {
        return Err("该文件夹第一层没有视频文件".to_string());
    }
    let kw = o
        .kw
        .clone()
        .or_else(|| {
            if folder_p.title.trim().is_empty() {
                None
            } else {
                Some(folder_p.title.clone())
            }
        })
        .or_else(|| parse::majority_title(&items))
        .unwrap_or_default();

    // 中文名：显式指定 > 本地缓存 > 在线查询
    let mut cache = bgm::Cache::load();
    let cn = match o.cn.clone() {
        Some(c) => {
            if !o.json {
                println!("使用指定中文名：{c}");
            }
            c
        }
        None => {
            let proxy = if o.direct {
                None
            } else {
                o.proxy
                    .clone()
                    .or_else(|| proxy::detect().url)
            };
            if !o.json {
                let how = match &proxy {
                    Some(p) => format!("代理 {p}"),
                    None => "直连".to_string(),
                };
                println!("查询 Bangumi：「{kw}」（{how}）");
            }
            let out = bgm::lookup(&kw, proxy.as_deref(), &mut cache, !o.no_cache);
            if let Some(e) = &out.error {
                return Err(format!("{e}\n（可用 --cn 中文名 手动指定）"));
            }
            if !o.json {
                for (n, c) in out.cands.iter().take(5).enumerate() {
                    println!(
                        "  {}. {} | {} → {} | 分 {:.2} | {}",
                        n + 1,
                        c.id,
                        c.name,
                        if c.name_cn.is_empty() { "—" } else { &c.name_cn },
                        c.score,
                        c.date
                    );
                }
            }
            out.cands
                .first()
                .and_then(|c| c.best_cn())
                .ok_or_else(|| format!("Bangumi 未找到「{kw}」的中文名，请用 --cn 指定"))?
        }
    };

    let plan = rename::build_plan(&dir, &cn, &o.opts)?;

    if o.json {
        let v = serde_json::json!({
            "dir": plan.dir.to_string_lossy(),
            "cn": cn,
            "keyword": kw,
            "folder_old": plan.folder_old,
            "folder_new": plan.folder_new,
            "rows": plan.rows.iter().map(|r| serde_json::json!({
                "old": r.old, "new": r.new, "ep": r.ep,
                "kind": format!("{:?}", r.kind), "note": r.note,
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
    } else {
        println!("中文片名：{cn}");
        if let Some(f) = &plan.folder_new {
            println!("文件夹：{}  →  {}", plan.folder_old, f);
        } else {
            println!("文件夹：{} （无需修改）", plan.folder_old);
        }
        for r in &plan.rows {
            let mark = if r.old == r.new { "=" } else { "→" };
            println!("  {mark} {}", r.old);
            if r.old != r.new {
                println!("     {}", r.new);
            }
            if !r.note.is_empty() {
                println!("     ({})", r.note);
            }
        }
        if !plan.skipped.is_empty() {
            println!("保持原样：{}", plan.skipped.join("、"));
        }
        println!("共 {} 项变更", plan.change_count());
    }

    if o.apply {
        let log = rename::apply(&plan, o.rename_folder);
        for l in log {
            println!("{l}");
        }
    } else if !o.json {
        println!("（预览模式，加 --apply 才会真正改名）");
    }
    Ok(())
}
