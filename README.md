# AnimeRename

把动漫文件夹和文件名批量重命名为中文的 Windows 桌面程序。基于 Rust + 原生 Win32 控件，
单文件免安装，中文片名来自 [Bangumi](https://bgm.tv)（番剧收藏维基）。

```
[MingY] Shuumatsu Touring [12][WebRip][JPSC].mkv
  ↓
末世二轮之旅 - 12 [WebRip][JPSC].mkv

[MingY] Shuumatsu Touring [WebRip][JPSC]        ← 文件夹
  ↓
末世二轮之旅 [WebRip][JPSC]
```

## 下载

到 [Releases](../../releases) 下载 `AnimeRename.exe`，双击即可运行，不需要安装运行时
（只链接系统 DLL）。

## 使用

### 图形界面

1. 点「浏览…」选择动漫文件夹，或直接把文件夹拖到窗口上。
2. 程序解析文件名、推断系列原名，并向 Bangumi 查询中文片名。
3. 在预览表里核对结果，可改「中文片名」或在「候选条目」下拉框里换一条，表格即时刷新。
4. 点「应用改名」。

改错了点「撤销上次改名」即可还原（含文件夹本身）。

### 命令行

```bash
AnimeRename.exe --cli <文件夹>          # 预览
AnimeRename.exe --cli <文件夹> --apply  # 执行
AnimeRename.exe --cli --undo            # 撤销最近一次
```

| 选项 | 说明 |
| --- | --- |
| `--cn <中文名>` | 跳过在线查询，直接指定中文片名 |
| `--kw <查询词>` | 覆盖自动推断的系列原名 |
| `--proxy <地址>` | 指定代理，如 `http://127.0.0.1:7890` |
| `--direct` | 强制直连，忽略系统代理 |
| `--no-cache` | 忽略本地查询缓存 |
| `--keep-group` | 保留字幕组前缀，如 `[MingY]` |
| `--no-folder` | 不改文件夹名 |
| `--json` | 以 JSON 输出改名计划 |

## 解析规则

支持常见的发布组命名：

```
[MingY] Shuumatsu Touring [12][WebRip][JPSC].mkv
[Airota] Sousou no Frieren [04][1080p][x265 AAC].mkv
Sousou no Frieren - 05 [1080p].mkv
某动画 第12话 [GB].mkv
[MingY] Show [03v2][WebRip].mkv
```

- 集数识别：`[12]`、`- 12`、`S01E12`、`第12话`、`[12v2]`；`[2025]` 这类四位年份不会被误判为集数。
- 字幕组前缀默认丢弃，`.ass` / `.srt` 等同名附属文件跟随视频一起改名。
- 分辨率、音轨等尾部标签原样保留。
- 未知类型文件（如 `说明.txt`、封面图）保持原名。

## 中文片名怎么来的

调用 Bangumi 的公开搜索接口：

```
POST https://api.bgm.tv/v0/search/subjects
{ "keyword": "Shuumatsu Touring", "type": 2, "max_results": 25 }
```

注意 `GET /v0/subjects?keyword=` 的 `keyword` 参数是无效的（会返回全量列表），
只有上面的 POST 形式才真正按关键词过滤，且无需鉴权。

排序不是简单取第一条。Bangumi 返回顺序本身带相关性，而查询词常是罗马音、命中的却是
日文条目名，两者字面相似度接近 0，因此打分只做「小幅加、明显减」：非动画类型、
OST／原声碟、缺少中文名的条目会被扣分，同时保留返回位置作为先验。这样
「葬送のフリーレン」不会因为 OST 排在前面而被错选。所有候选都会列在界面上，
自动结果不满意时可以直接换。

查询结果会缓存到 `%LOCALAPPDATA%\AnimeRename\cache.json`，下次离线也能用。

## 代理

默认跟随 Windows 系统代理，优先级为：界面手动填写 → 环境变量 →
注册表 `HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings`。

实测经代理的首次 CONNECT 偶发失败，因此请求会按「系统代理 ×2 → 直连 ×2」重试降级。
若系统配置的是 PAC 自动代理脚本，程序无法解析其内容，会在日志里提示手动填写。

## 界面与高 DPI

布局坐标按 96 DPI 逻辑像素书写，建窗后读取该窗口的真实 DPI
（`GetDpiForWindow`，因为 Per-Monitor V2 进程里 `GetDeviceCaps` 只会返回系统 DPI），
再统一换算成物理像素；字号以「磅」指定（`FS_PT`）。因此在 125%／150% 缩放下
文字都是锐利的，程序启动时会在日志里自报缩放比例便于核对。

## 构建

```bash
cargo build --release    # 产物 target/release/AnimeRename.exe
cargo test               # 20 项单元测试
```

依赖只有 `native-windows-gui`、`ureq`（native-tls，走系统 schannel，不需要 C 工具链编译依赖）、
`serde`、`serde_json`、`regex`、`winreg`、`rfd`、`chrono`、`unicode-normalization`。

Windows 上若用 GNU 工具链，需要一个 MinGW-w64 提供 `gcc`/`dlltool`
（`windows-sys` 在 GNU 目标下链接需要 `dlltool`）。在项目下的 `.cargo/config.toml` 里指定即可，
不必改系统 PATH——该文件含本机绝对路径，已在 `.gitignore` 中排除：

```toml
[target.x86_64-pc-windows-gnu]
linker = "C:/tools/mingw64/bin/gcc.exe"
ar     = "C:/tools/mingw64/bin/ar.exe"

[env]
PATH = { value = "C:\tools\mingw64\bin;{PATH}", force = false }
```

MSVC 工具链则装好 Visual Studio Build Tools 即可，无需以上配置。
国内网络建议把 rustup 与 crates 源换成镜像（如 `rsproxy.cn`）。

## 已知限制

- 只处理所选文件夹的**第一层**，不递归子目录；若发现子目录里有视频会在日志中提示。
- 改名要求目标文件未被播放器占用；被占用时该条会失败并在日志中说明，其余继续执行。
- 目标名冲突时自动追加 ` (2)` 之类的序号，不会覆盖已有文件。

## 隐私

程序只把解析出的系列原名发送到 bgm.tv 用于查询，不发送完整文件路径或文件名。
改名操作全部在本地完成。

## 许可证

暂未指定。
