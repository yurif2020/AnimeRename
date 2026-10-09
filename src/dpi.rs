// SPDX-License-Identifier: GPL-3.0-only
// SPDX-FileCopyrightText: 2026 yurif2020
//! 高 DPI 支持
//!
//! 没有启用 native-windows-gui 的 `high-dpi` 特性：它只会自动缩放 ListView 的
//! 列宽（list_view.rs 里 `width * scale_factor()`），却不缩放窗口和子控件坐标，
//! 混用会让列宽被放大两次。这里自己声明 DPI 感知、自己取 DPI、由布局层统一换算。
//!
//! 注意取 DPI 的方式：在 Per-Monitor V2 进程里，`GetDeviceCaps(LOGPIXELSX)`
//! 返回的是**系统** DPI（本机实测 96），而窗口所在显示器实际是 120 DPI（125%）。
//! 所以必须用 `GetDpiForWindow(hwnd)`，也就是先建窗、再按该窗口的 DPI 布局。

use std::sync::atomic::{AtomicI32, Ordering};

static DPI: AtomicI32 = AtomicI32::new(96);

#[cfg(windows)]
mod win {
    use std::ffi::c_void;

    #[link(name = "user32")]
    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const i8) -> Option<unsafe extern "system" fn()>;
        fn SetProcessDPIAware() -> i32;
    }

    /// 按名字取导出函数。直接静态链接这些较新的 API 会让程序在旧系统上无法启动。
    pub unsafe fn u32fn(name: &'static [u8]) -> Option<unsafe extern "system" fn()> {
        let mut mod_name: Vec<u16> = "user32".encode_utf16().chain(std::iter::once(0)).collect();
        let module = GetModuleHandleW(mod_name.as_mut_ptr());
        if module.is_null() {
            return None;
        }
        let mut fname: Vec<i8> = name.iter().map(|b| *b as i8).collect();
        fname.push(0);
        GetProcAddress(module, fname.as_ptr())
    }

    pub unsafe fn set_process_dpi_aware() -> bool {
        const PMV2: isize = -4;
        type FnSetCtx = unsafe extern "system" fn(isize) -> i32;
        if let Some(addr) = u32fn(b"SetProcessDpiAwarenessContext") {
            let f: FnSetCtx = std::mem::transmute(addr);
            if f(PMV2) != 0 {
                return true;
            }
        }
        SetProcessDPIAware() != 0
    }

    pub unsafe fn dpi_for_window(hwnd: isize) -> Option<i32> {
        type FnGet = unsafe extern "system" fn(*mut c_void) -> u32;
        let addr = u32fn(b"GetDpiForWindow")?;
        let f: FnGet = std::mem::transmute(addr);
        let d = f(hwnd as *mut c_void);
        if d > 0 {
            Some(d as i32)
        } else {
            None
        }
    }
}

/// 声明进程 DPI 感知。必须在创建任何窗口之前调用。
pub fn make_aware() {
    #[cfg(windows)]
    unsafe {
        win::set_process_dpi_aware();
    }
}

/// 用窗口所在显示器的真实 DPI 刷新全局比例；建窗后、布局前调用
pub fn sync_to_window(hwnd: isize) {
    #[cfg(windows)]
    unsafe {
        if let Some(d) = win::dpi_for_window(hwnd) {
            DPI.store(d, Ordering::Relaxed);
        }
    }
    let _ = hwnd;
}

pub fn dpi() -> i32 {
    DPI.load(Ordering::Relaxed)
}

/// 相对 96 DPI 的缩放系数
pub fn scale() -> f64 {
    f64::from(dpi()) / 96.0
}

/// 96DPI 逻辑尺寸 → 物理像素
pub fn px(logical: i32) -> i32 {
    (logical as f64 * scale()).round() as i32
}

/// 磅 → 字身高度像素（CreateFontW 的负 nHeight）
pub fn em_from_pt(pt: f64) -> i32 {
    (pt * f64::from(dpi()) / 72.0).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        assert!(dpi() >= 96);
        assert!(scale() >= 1.0);
        assert_eq!(px(100), (100.0 * scale()).round() as i32);
        assert!(em_from_pt(11.0) >= 14);
    }
}
