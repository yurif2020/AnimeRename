// SPDX-License-Identifier: GPL-3.0-only
// SPDX-FileCopyrightText: 2026 yurif2020
//! 图形界面（native-windows-gui，原生 Win32 控件）

use crate::bgm;
use crate::parse::{self, RenameOpts};
use crate::proxy;
use crate::rename::{self, Kind};
use native_windows_gui as nwg;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Default)]
pub struct AppState {
    pub dir: PathBuf,
    pub keyword: String,
    pub plan: Option<rename::Plan>,
    /// 程序内部写入中文名片段时，用它抑制 OnTextInput 造成的重复预览
    pub suppress: bool,
}

struct Ctl {
    window: nwg::Window,
    ed_dir: nwg::TextInput,
    btn_browse: nwg::Button,
    btn_preview: nwg::Button,
    ed_kw: nwg::TextInput,
    btn_query: nwg::Button,
    ed_proxy: nwg::TextInput,
    chk_direct: nwg::CheckBox,
    chk_cache: nwg::CheckBox,
    ed_cn: nwg::TextInput,
    chk_folder: nwg::CheckBox,
    chk_group: nwg::CheckBox,
    lbl_net: nwg::Label,
    combo: nwg::ComboBox<String>,
    btn_use_cand: nwg::Button,
    list: nwg::ListView,
    ed_row: nwg::TextInput,
    btn_row: nwg::Button,
    btn_apply: nwg::Button,
    btn_undo: nwg::Button,
    lbl_status: nwg::Label,
    log: nwg::TextBox,
    /// 所有纯展示用的标签都要被持有，否则 Drop 时窗口句柄会被销毁
    labels: Vec<nwg::Label>,
}

type Shared = Rc<RefCell<AppState>>;
type UI = Rc<Ctl>;

/// 启动 GUI
pub fn run(init_dir: Option<PathBuf>) -> Result<(), nwg::NwgError> {
    // 先声明 DPI 感知，再初始化；否则系统会按 96DPI 渲染后整体拉伸，文字发虚
    crate::dpi::make_aware();
    nwg::init()?;

    let sys = proxy::detect();
    let st: Shared = Rc::new(RefCell::new(AppState::default()));
    if let Some(d) = init_dir {
        st.borrow_mut().dir = d;
    }

    let ctl: UI = Rc::new(build(&sys, &st)?);

    let c2 = ctl.clone();
    let s2 = st.clone();
    let handler = nwg::full_bind_event_handler(&ctl.window.handle, move |evt, data, handle| {
        use nwg::Event as E;
        match evt {
            E::OnButtonClick => {
                if &handle == &c2.btn_browse {
                    c2.browse(&s2);
                } else if &handle == &c2.btn_preview {
                    c2.preview(&s2);
                } else if &handle == &c2.btn_query {
                    c2.query(&s2);
                } else if &handle == &c2.btn_use_cand {
                    c2.use_candidate(&s2);
                } else if &handle == &c2.btn_row {
                    c2.override_selected(&s2);
                } else if &handle == &c2.btn_apply {
                    c2.apply(&s2);
                } else if &handle == &c2.btn_undo {
                    c2.undo(&s2);
                }
            }
            E::OnComboxBoxSelection => {
                if &handle == &c2.combo {
                    c2.use_candidate(&s2);
                }
            }
            E::OnTextInput => {
                if &handle == &c2.ed_cn && !s2.borrow().suppress {
                    c2.rebuild(&s2);
                }
            }
            E::OnFileDrop => {
                if let Some(first) = data.on_file_drop().files().first().cloned() {
                    let p = PathBuf::from(first);
                    let dir = if p.is_dir() {
                        p
                    } else {
                        p.parent().map(|x| x.to_path_buf()).unwrap_or(p)
                    };
                    c2.set_dir(&s2, dir);
                    c2.preview(&s2);
                }
            }
            E::OnWindowClose => {
                nwg::stop_thread_dispatch();
            }
            _ => {}
        }
    });

    ctl.log_line(&format!(
        "就绪。选择动漫文件夹后点「解析并预览」；也可以把文件夹直接拖进本窗口。\n{}\n",
        sys.desc
    ));
    match &sys.url {
        Some(u) => ctl.log_line(&format!("查询将通过系统代理 {u} 发出（可在上方修改，或勾选「强制直连」）。")),
        None => match &sys.pac {
            Some(pac) => ctl.log_line(&format!(
                "系统配置的是 PAC 自动代理（{pac}），无法自动解析。
若查询失败，请在「代理」里手工填写地址（如 http://127.0.0.1:7890）。"
            )),
            None => ctl.log_line("未检测到系统代理，将直接连接。"),
        },
    }

    if !st.borrow().dir.as_os_str().is_empty() {
        ctl.preview(&st);
    }
    // 界面缩放自检信息放在最后，保证在日志里可见
    let (ww, wh) = ctl.window.size();
    ctl.log_line(&format!(
        "界面缩放 {:.0}%（DPI {}）· 字号 {} 磅 · 窗口 {ww}x{wh} 物理像素",
        crate::dpi::scale() * 100.0,
        crate::dpi::dpi(),
        FS_PT
    ));

    nwg::dispatch_thread_events();
    drop(handler);
    Ok(())
}

// ---------------------------------------------------------------- 布局
//
// 坐标一律按 96DPI 的「逻辑像素」书写，只在交给 Win32 的最后一步用 dpi::px()
// 换算成物理像素；字号用真实的「磅」指定。这样在高 DPI 屏上文字和控件都会
// 按比例变大且是像素级锐利的。控件通过 Line::take() 领取互不重叠的矩形。

const FS_PT: f64 = 11.0; // 字号（磅）

const M: i32 = 12;          // 窗口内边距
const G: i32 = 8;           // 控件水平间距
const VG: i32 = 7;          // 行垂直间距
const CONTENT: i32 = 900;   // 内容区宽度（逻辑像素）

const HH: i32 = 26;         // 分区标题行
const RH: i32 = 32;         // 普通行
const LH: i32 = 20;         // 标签控件高
const IH: i32 = 26;         // 输入框控件高
const BH: i32 = 28;         // 按钮控件高
const CH: i32 = 22;         // 复选框 / 下拉框控件高
const HINT_H: i32 = 18;     // 提示行
const LIST_H: i32 = 200;    // 预览表
const LOG_H: i32 = 92;      // 日志

const LW: i32 = 68;         // 字段标签宽
const BW: i32 = 118;        // 通用按钮宽
const QW: i32 = 112;        // 「查询中文名」宽
const APW: i32 = 146;       // 「应用改名」宽
const PW: i32 = 210;        // 代理输入框宽
const CBW: i32 = 170;       // 复选框宽

/// 总高度由各行高累加得出，改任何常量都不会让窗口尺寸失配
const BODY_H: i32 = 3 * HH + 6 * RH + (RH + 6) + HINT_H + 2 + LIST_H + LOG_H;
const BODY_ROWS: i32 = 14;
const CLIENT_H: i32 = BODY_H + (BODY_ROWS - 1) * VG + 2 * M;

const BAND: [u8; 3] = [222, 234, 250]; // 分区标题条
const RULE: [u8; 3] = [198, 210, 226]; // 细分隔线

struct Sheet {
    y: i32,
}

impl Sheet {
    fn new() -> Sheet {
        Sheet { y: M }
    }
    fn line(&mut self, h: i32) -> Line {
        let l = Line {
            x: M,
            y: self.y,
            w: CONTENT,
            h,
        };
        self.y += h + VG;
        l
    }
    fn block(&mut self, h: i32) -> Cell {
        let c = Cell {
            x: M,
            y: self.y,
            w: CONTENT,
            h,
        };
        self.y += h + VG;
        c
    }
}

struct Line {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl Line {
    fn take(&mut self, w: i32) -> Cell {
        let c = Cell {
            x: self.x,
            y: self.y,
            w,
            h: self.h,
        };
        self.x += w + G;
        self.w -= w + G;
        c
    }
    fn rest(&mut self) -> Cell {
        let w = self.w.max(60);
        self.take(w)
    }
}

#[derive(Clone, Copy)]
struct Cell {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl Cell {
    /// 逻辑坐标下垂直居中
    fn centered(&self, ch: i32) -> (i32, i32, i32, i32) {
        let off = (self.h - ch).max(0) / 2;
        (self.x, self.y + off, self.w, ch)
    }
}

fn mk_label(
    parent: &nwg::Window,
    cell: Cell,
    text: &str,
    h: i32,
    bg: Option<[u8; 3]>,
) -> Result<nwg::Label, nwg::NwgError> {
    let (x, y, w, h) = cell.centered(h);
    let mut l = nwg::Label::default();
    let mut b = nwg::Label::builder()
        .parent(parent)
        .position((crate::dpi::px(x), crate::dpi::px(y)))
        .size((crate::dpi::px(w), crate::dpi::px(h)))
        .text(text);
    if let Some(c) = bg {
        b = b.background_color(Some(c));
    }
    b.build(&mut l)?;
    Ok(l)
}

fn mk_input(
    parent: &nwg::Window,
    cell: Cell,
    text: &str,
    out: &mut nwg::TextInput,
) -> Result<(), nwg::NwgError> {
    let (x, y, w, h) = cell.centered(IH);
    nwg::TextInput::builder()
        .parent(parent)
        .position((crate::dpi::px(x), crate::dpi::px(y)))
        .size((crate::dpi::px(w), crate::dpi::px(h)))
        .text(text)
        .build(out)
}

fn mk_button(
    parent: &nwg::Window,
    cell: Cell,
    text: &str,
    out: &mut nwg::Button,
) -> Result<(), nwg::NwgError> {
    let (x, y, w, h) = cell.centered(BH);
    nwg::Button::builder()
        .parent(parent)
        .position((crate::dpi::px(x), crate::dpi::px(y)))
        .size((crate::dpi::px(w), crate::dpi::px(h)))
        .text(text)
        .build(out)
}

fn mk_check(
    parent: &nwg::Window,
    cell: Cell,
    text: &str,
    checked: bool,
    out: &mut nwg::CheckBox,
) -> Result<(), nwg::NwgError> {
    let (x, y, w, h) = cell.centered(CH);
    nwg::CheckBox::builder()
        .parent(parent)
        .position((crate::dpi::px(x), crate::dpi::px(y)))
        .size((crate::dpi::px(w), crate::dpi::px(h)))
        .text(text)
        .check_state(if checked {
            nwg::CheckBoxState::Checked
        } else {
            nwg::CheckBoxState::Unchecked
        })
        .build(out)
}

fn build(sys: &proxy::ProxyInfo, st: &Shared) -> Result<Ctl, nwg::NwgError> {
    let mut c = Ctl {
        window: nwg::Window::default(),
        ed_dir: nwg::TextInput::default(),
        btn_browse: nwg::Button::default(),
        btn_preview: nwg::Button::default(),
        ed_kw: nwg::TextInput::default(),
        btn_query: nwg::Button::default(),
        ed_proxy: nwg::TextInput::default(),
        chk_direct: nwg::CheckBox::default(),
        chk_cache: nwg::CheckBox::default(),
        ed_cn: nwg::TextInput::default(),
        chk_folder: nwg::CheckBox::default(),
        chk_group: nwg::CheckBox::default(),
        lbl_net: nwg::Label::default(),
        combo: nwg::ComboBox::default(),
        btn_use_cand: nwg::Button::default(),
        list: nwg::ListView::default(),
        ed_row: nwg::TextInput::default(),
        btn_row: nwg::Button::default(),
        btn_apply: nwg::Button::default(),
        btn_undo: nwg::Button::default(),
        lbl_status: nwg::Label::default(),
        log: nwg::TextBox::default(),
        labels: Vec::new(),
    };

    // 先建窗（占位尺寸），才能问到该窗口所在显示器的真实 DPI
    nwg::Window::builder()
        .flags(nwg::WindowFlags::MAIN_WINDOW | nwg::WindowFlags::VISIBLE)
        .size((640, 480))
        .position((24, 12))
        .title("动漫文件名中文重命名 · Bangumi")
        .accept_files(true)
        .build(&mut c.window)?;

    if let Some(h) = c.window.handle.hwnd() {
        crate::dpi::sync_to_window(h as isize);
    }
    let px = crate::dpi::px;
    c.window.set_size(
        (px(CONTENT + 2 * M) + 16) as u32,
        (px(CLIENT_H) + 44) as u32,
    );

    // 全局字体必须在拿到 DPI 之后、创建子控件之前设置：
    // size_absolute 传的是字身高度像素，11 磅在 120DPI 屏上即 18 像素。
    let em = crate::dpi::em_from_pt(FS_PT);
    let mut f_body = nwg::Font::default();
    nwg::Font::builder()
        .size_absolute(em as u32)
        .family("Microsoft YaHei UI")
        .build(&mut f_body)?;
    nwg::Font::set_global_default(Some(f_body));

    let mut sh = Sheet::new();

    // ---------- ① 目标文件夹 ----------
    let mut l = sh.line(HH);
    c.labels.push(mk_label(
        &c.window,
        l.rest(),
        "  ① 目标文件夹",
        HH,
        Some(BAND),
    )?);
    let mut l = sh.line(RH);
    let init_dir: String = st.borrow().dir.to_string_lossy().to_string();
    mk_input(
        &c.window,
        l.take(CONTENT - 2 * (BW + G)),
        &init_dir,
        &mut c.ed_dir,
    )?;
    mk_button(&c.window, l.take(BW), "浏览…", &mut c.btn_browse)?;
    mk_button(&c.window, l.rest(), "解析并预览", &mut c.btn_preview)?;

    // ---------- ② 中文译名 ----------
    let mut l = sh.line(HH);
    c.labels.push(mk_label(
        &c.window,
        l.rest(),
        "  ② 中文译名 · Bangumi（默认使用系统代理）",
        HH,
        Some(BAND),
    )?);
    let mut l = sh.line(RH);
    c.labels.push(mk_label(&c.window, l.take(LW), "系列原名", LH, None)?);
    mk_input(&c.window, l.take(260), "", &mut c.ed_kw)?;
    mk_button(&c.window, l.take(QW), "查询中文名", &mut c.btn_query)?;
    c.labels.push(mk_label(&c.window, l.take(LW), "中文片名", LH, None)?);
    mk_input(&c.window, l.rest(), "", &mut c.ed_cn)?;

    let mut l = sh.line(RH);
    c.labels.push(mk_label(&c.window, l.take(LW), "候选条目", LH, None)?);
    let (cx, cy, cw, chh) = l.take(CONTENT - LW - BW - 2 * G).centered(CH);
    nwg::ComboBox::builder()
        .parent(&c.window)
        .position((px(cx), px(cy)))
        .size((px(cw), px(240))) // 高度是展开后的列表高度
        .collection(Vec::<String>::new())
        .build(&mut c.combo)?;
    let _ = chh;
    mk_button(&c.window, l.rest(), "采用此候选", &mut c.btn_use_cand)?;

    let mut l = sh.line(RH);
    c.labels.push(mk_label(&c.window, l.take(LW), "输出选项", LH, None)?);
    mk_check(
        &c.window,
        l.take(CBW),
        "同时重命名文件夹",
        true,
        &mut c.chk_folder,
    )?;
    mk_check(
        &c.window,
        l.take(CBW),
        "保留字幕组前缀",
        false,
        &mut c.chk_group,
    )?;
    c.lbl_net = mk_label(&c.window, l.rest(), "网络：待查询", LH, None)?;

    let mut l = sh.line(RH);
    c.labels.push(mk_label(&c.window, l.take(LW), "代理", LH, None)?);
    mk_input(
        &c.window,
        l.take(PW),
        sys.url.as_deref().unwrap_or(""),
        &mut c.ed_proxy,
    )?;
    mk_check(
        &c.window,
        l.take(110),
        "强制直连",
        false,
        &mut c.chk_direct,
    )?;
    mk_check(
        &c.window,
        l.take(122),
        "优先用缓存",
        true,
        &mut c.chk_cache,
    )?;
    c.labels.push(mk_label(
        &c.window,
        l.rest(),
        "留空即自动跟随系统代理",
        HINT_H,
        None,
    )?);

    // ---------- ③ 改名预览 ----------
    let mut l = sh.line(HH);
    c.labels.push(mk_label(
        &c.window,
        l.rest(),
        "  ③ 改名预览（改中文片名或换候选，表格即时刷新）",
        HH,
        Some(BAND),
    )?);
    let lc = sh.block(LIST_H);
    nwg::ListView::builder()
        .parent(&c.window)
        .position((px(lc.x), px(lc.y)))
        .size((px(lc.w), px(lc.h)))
        .item_count(64)
        .list_style(nwg::ListViewStyle::Detailed)
        .ex_flags(nwg::ListViewExFlags::GRID | nwg::ListViewExFlags::FULL_ROW_SELECT)
        .flags(
            nwg::ListViewFlags::VISIBLE
                | nwg::ListViewFlags::TAB_STOP
                | nwg::ListViewFlags::SINGLE_SELECTION
                | nwg::ListViewFlags::ALWAYS_SHOW_SELECTION,
        )
        .build(&mut c.list)?;
    for t in ["原文件名", "新文件名", "集数", "说明"] {
        c.list.insert_column(t);
    }
    // 列宽按内容宽度分配，最后一列吃掉余量，保证不出现横向滚动
    let cols = [px(330), px(330), px(46)];
    let mut used = 0;
    for (i, w) in cols.iter().enumerate() {
        c.list.set_column_width(i, *w as isize);
        used += *w;
    }
    c.list
        .set_column_width(3, (px(CONTENT) - used).max(px(90)) as isize);

    let mut l = sh.line(RH);
    mk_input(
        &c.window,
        l.take(CONTENT - 2 * (BW + G)),
        "选中某行后，在这里填入完整新文件名",
        &mut c.ed_row,
    )?;
    mk_button(&c.window, l.take(BW), "覆盖选中行", &mut c.btn_row)?;
    mk_button(&c.window, l.rest(), "撤销上次改名", &mut c.btn_undo)?;

    let mut l = sh.line(HINT_H);
    c.labels.push(mk_label(
        &c.window,
        l.rest(),
        "  提示：只处理该文件夹第一层，不递归子目录；字幕等同名附属文件跟随视频一起改名。",
        HINT_H,
        None,
    )?);

    // ---------- ④ 执行 ----------
    let mut l = sh.line(2);
    c.labels.push(mk_label(&c.window, l.rest(), "", 2, Some(RULE))?);
    let mut l = sh.line(RH + 6);
    mk_button(&c.window, l.take(APW), "应用改名", &mut c.btn_apply)?;
    c.lbl_status = mk_label(&c.window, l.rest(), "尚未解析", LH, None)?;

    let gc = sh.block(LOG_H);
    nwg::TextBox::builder()
        .parent(&c.window)
        .position((px(gc.x), px(gc.y)))
        .size((px(gc.w), px(gc.h)))
        .readonly(true)
        .build(&mut c.log)?;

    Ok(c)
}

// ---------------------------------------------------------------- 交互逻辑

impl Ctl {
    fn log_line(&self, s: &str) {
        self.log.appendln(s);
    }

    fn checked(&self, b: &nwg::CheckBox) -> bool {
        b.check_state() == nwg::CheckBoxState::Checked
    }

    fn opts(&self) -> RenameOpts {
        RenameOpts {
            keep_group: self.checked(&self.chk_group),
            keep_lead_tags: false,
        }
    }

    /// 手动填写 > 系统代理 > 直连
    fn proxy_url(&self) -> Option<String> {
        if self.checked(&self.chk_direct) {
            return None;
        }
        let manual = self.ed_proxy.text().trim().to_string();
        if !manual.is_empty() {
            return proxy::normalize_server(&manual);
        }
        proxy::detect().url
    }

    fn dir_of(&self) -> PathBuf {
        PathBuf::from(self.ed_dir.text().trim().trim_matches('"'))
    }

    fn cn_of(&self) -> String {
        self.ed_cn.text().trim().to_string()
    }

    fn set_dir(&self, st: &Shared, dir: PathBuf) {
        self.ed_dir.set_text(&dir.to_string_lossy());
        st.borrow_mut().dir = dir;
    }

    fn set_cn(&self, st: &Shared, cn: &str) {
        st.borrow_mut().suppress = true;
        self.ed_cn.set_text(cn);
        st.borrow_mut().suppress = false;
    }

    fn browse(&self, st: &Shared) {
        let mut dlg = rfd::FileDialog::new().set_title("选择动漫文件夹");
        let cur = st.borrow().dir.clone();
        if !cur.as_os_str().is_empty() && cur.exists() {
            dlg = dlg.set_directory(&cur);
        }
        if let Some(p) = dlg.pick_folder() {
            self.set_dir(st, p);
            self.preview(st);
        }
    }

    /// 解析目录，必要时自动查询，最后生成预览
    fn preview(&self, st: &Shared) {
        let dir = self.dir_of();
        if let Err(e) = rename::check_dir(&dir) {
            nwg::modal_error_message(&self.window, "路径有误", &e);
            return;
        }
        st.borrow_mut().dir = dir.clone();

        let folder_p = parse::parse_name(&name_of(&dir));
        let mut vids = Vec::new();
        let mut total = 0usize;
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let p = e.path();
                if !p.is_file() {
                    continue;
                }
                total += 1;
                let (stem, ext) = parse::split_ext(&p.file_name().unwrap_or_default().to_string_lossy());
                if parse::is_video(&ext) {
                    vids.push(parse::parse_name(&stem));
                }
            }
        }
        if total == 0 {
            nwg::modal_info_message(
                &self.window,
                "没有可处理的文件",
                "该文件夹的第一层里没有文件。本工具只处理当前这一层，不递归子目录。",
            );
            return;
        }

        let keyword = if folder_p.title.trim().is_empty() {
            parse::majority_title(&vids).unwrap_or_default()
        } else {
            folder_p.title.clone()
        };
        self.ed_kw.set_text(&keyword);
        st.borrow_mut().keyword = keyword.clone();

        if !vids.is_empty() && vids.iter().all(|p| p.episode.is_none()) && vids.len() > 1 {
            self.log_line("提示：这些文件名里都没找到集数标记，请留意预览结果。");
        }

        if self.cn_of().is_empty() && !keyword.trim().is_empty() {
            self.query(st);
        } else {
            self.rebuild(st);
        }
    }

    fn query(&self, st: &Shared) {
        let kw = self.ed_kw.text().trim().to_string();
        if kw.is_empty() {
            nwg::modal_info_message(&self.window, "缺少查询词", "请先填写「系列原名」。");
            return;
        }
        let p = self.proxy_url();
        self.lbl_net.set_text("网络：查询中…");
        self.log_line(&format!(
            "正在查询 Bangumi：「{kw}」（{}）…",
            match &p {
                Some(u) => format!("代理 {u}"),
                None => "直连".to_string(),
            }
        ));

        let mut cache = bgm::Cache::load();
        let use_cache = self.checked(&self.chk_cache);
        let out = bgm::lookup(&kw, p.as_deref(), &mut cache, use_cache);

        if let Some(e) = &out.error {
            self.lbl_net.set_text("网络：失败");
            self.log_line(&format!("查询失败：{e}"));
            self.log_line("可在「中文片名」里手动填写片名，再点「解析并预览」。");
            self.rebuild(st);
            return;
        }

        self.lbl_net.set_text(&format!(
            "网络：{} 条候选（{}）",
            out.cands.len(),
            out.via
        ));
        let top: Vec<String> = out
            .cands
            .iter()
            .take(3)
            .map(|c| {
                format!(
                    "{}[{:.2}]",
                    if c.name_cn.is_empty() { &c.name } else { &c.name_cn },
                    c.score
                )
            })
            .collect();
        self.log_line(&format!(
            "命中 {} 条候选（{}）：{}",
            out.cands.len(),
            if out.cached { "本地缓存" } else { "在线" },
            top.join("、")
        ));

        let texts: Vec<String> = out.cands.iter().map(|c| c.display()).collect();
        self.combo.set_collection(texts);
        if let Some(first) = out.cands.first() {
            self.combo.set_selection(Some(0));
            match first.best_cn() {
                Some(cn) => self.set_cn(st, &cn),
                None => self.log_line("最优候选缺少中文名，可在下拉框换一条，或手动填写。"),
            }
        }
        self.rebuild(st);
    }

    fn use_candidate(&self, st: &Shared) {
        let idx = match self.combo.selection() {
            Some(i) => i,
            None => return,
        };
        let text = match self.combo.selection_string() {
            Some(t) => t,
            None => return,
        };
        let cn = text.split(" — ").next().unwrap_or("").trim().to_string();
        if cn.is_empty() || cn == "（无中文名）" {
            self.log_line("这条候选没有中文名，请换一条。");
            return;
        }
        self.set_cn(st, &cn);
        self.log_line(&format!("已采用候选 {}：{cn}", idx + 1));
        self.rebuild(st);
    }

    fn rebuild(&self, st: &Shared) {
        let dir = self.dir_of();
        let cn = self.cn_of();
        if dir.as_os_str().is_empty() {
            return;
        }
        if cn.is_empty() {
            self.lbl_status.set_text("请先填写「中文片名」");
            return;
        }
        match rename::build_plan(&dir, &cn, &self.opts()) {
            Ok(plan) => {
                let videos = plan.video_rows().count();
                let side = plan.rows.iter().filter(|r| r.kind == Kind::Sidecar).count();
                let folder = if self.checked(&self.chk_folder) && plan.folder_new.is_some() {
                    " + 文件夹"
                } else {
                    ""
                };
                self.lbl_status
                    .set_text(&format!("预览：{videos} 个视频 + {side} 个附属文件{folder}"));
                if plan.subdirs_with_video > 0 {
                    self.log_line(&format!(
                        "注意：下面还有 {} 个子文件夹含视频，本工具只处理当前这一层。",
                        plan.subdirs_with_video
                    ));
                }
                if !plan.skipped.is_empty() {
                    self.log_line(&format!(
                        "保持原样（{} 个）：{}",
                        plan.skipped.len(),
                        plan.skipped.iter().take(6).cloned().collect::<Vec<_>>().join("、")
                    ));
                }
                self.render(&plan);
                st.borrow_mut().plan = Some(plan);
            }
            Err(e) => {
                self.lbl_status.set_text(&format!("解析失败：{e}"));
                self.log_line(&format!("解析失败：{e}"));
            }
        }
    }

    fn render(&self, plan: &rename::Plan) {
        self.list.clear();
        if let Some(f) = &plan.folder_new {
            let old = format!("【文件夹】{}", plan.folder_old);
            self.list
                .insert_items_row(None, &[old.as_str(), f.as_str(), "", "文件夹"]);
        }
        for r in &plan.rows {
            let tag = match r.kind {
                Kind::Video => "视频",
                Kind::Sidecar => "附属",
            };
            let mut note = r.note.clone();
            if r.old == r.new && note.is_empty() {
                note = "名称未变化".to_string();
            }
            let desc = format!("{tag} {}", note).trim().to_string();
            self.list.insert_items_row(
                None,
                &[
                    r.old.as_str(),
                    r.new.as_str(),
                    r.ep.as_deref().unwrap_or("-"),
                    desc.as_str(),
                ],
            );
        }
    }

    fn override_selected(&self, st: &Shared) {
        let row = match self.list.selected_item() {
            Some(i) => i,
            None => {
                self.log_line("请先在表格里选中一行。");
                return;
            }
        };
        let wanted = self.ed_row.text().trim().to_string();
        if wanted.is_empty() {
            return;
        }
        let mut ok = false;
        if let Some(plan) = st.borrow_mut().plan.as_mut() {
            let folder_row = plan.folder_new.is_some() && row == 0;
            let idx = if plan.folder_new.is_some() {
                row.saturating_sub(1)
            } else {
                row
            };
            if folder_row {
                plan.folder_new = Some(parse::sanitize_filename(&wanted));
                ok = true;
            } else if let Some(r) = plan.rows.get_mut(idx) {
                r.new = parse::sanitize_filename(&wanted);
                r.note = "手工指定".to_string();
                ok = true;
            }
        }
        if !ok {
            self.log_line("还没有预览结果，请先点「解析并预览」。");
            return;
        }
        let plan = st.borrow().plan.clone().unwrap();
        self.render(&plan);
    }

    fn apply(&self, st: &Shared) {
        let plan = match st.borrow().plan.clone() {
            Some(p) => p,
            None => {
                nwg::modal_info_message(&self.window, "没有预览", "请先点「解析并预览」生成改名方案。");
                return;
            }
        };
        let n = plan.change_count();
        if n == 0 {
            nwg::modal_info_message(&self.window, "无需操作", "所有名称已经是目标形式。");
            return;
        }
        let rename_folder = self.checked(&self.chk_folder);
        let msg = format!(
            "目录：{}\n\n将执行 {} 项重命名（{}）。\n\n撤销记录会自动保存，之后可点「撤销上次改名」恢复。\n确认执行？",
            plan.dir.display(),
            n,
            if rename_folder { "含文件夹本身" } else { "不含文件夹" }
        );
        let r = nwg::modal_message(
            &self.window,
            &nwg::MessageParams {
                title: "确认改名",
                content: &msg,
                buttons: nwg::MessageButtons::YesNo,
                icons: nwg::MessageIcons::Question,
            },
        );
        if r != nwg::MessageChoice::Yes {
            return;
        }
        for line in rename::apply(&plan, rename_folder) {
            self.log_line(&line);
        }
        if rename_folder {
            if let Some(new) = &plan.folder_new {
                if let Some(parent) = plan.dir.parent() {
                    let np = parent.join(new);
                    if np.exists() {
                        self.set_dir(st, np);
                    }
                }
            }
        }
        self.rebuild(st);
        self.log_line("完成。");
    }

    fn undo(&self, st: &Shared) {
        let j = match rename::load_journal() {
            Some(j) => j,
            None => {
                nwg::modal_info_message(&self.window, "没有记录", "本程序还没有执行过改名操作。");
                return;
            }
        };
        let r = nwg::modal_message(
            &self.window,
            &nwg::MessageParams {
                title: "确认撤销",
                content: &format!(
                    "将还原 {} 项操作：\n{}\n（记录于 {}）\n\n确认撤销？",
                    j.ops.len(),
                    j.dir,
                    crate::util::fmt_ts(j.ts)
                ),
                buttons: nwg::MessageButtons::YesNo,
                icons: nwg::MessageIcons::Warning,
            },
        );
        if r != nwg::MessageChoice::Yes {
            return;
        }
        for line in rename::undo(&j) {
            self.log_line(&line);
        }
        let p = PathBuf::from(&j.dir);
        let base = if p.exists() {
            p
        } else {
            // 文件夹可能已被改名，退回记录里的新名
            PathBuf::from(&j.ops[0].old)
                .parent()
                .map(|x| x.to_path_buf())
                .unwrap_or(p)
        };
        if base.exists() {
            self.set_dir(st, base);
        }
        self.rebuild(st);
    }
}

fn name_of(p: &std::path::Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}
