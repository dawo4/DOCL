use tauri::WebviewWindow;

/// Windows 11 的 build 号从 22000 开始；读不到版本时按 Win11 处理，避免误降级。
#[cfg(windows)]
fn is_windows_11() -> bool {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        .and_then(|key| key.get_value::<String, _>("CurrentBuildNumber"))
        .ok()
        .and_then(|build| build.parse::<u32>().ok())
        .map(|build| build >= 22000)
        .unwrap_or(true)
}

#[cfg(not(windows))]
fn is_windows_11() -> bool {
    false
}

/// 开启毛玻璃。
/// Win11 用系统原生 Acrylic；Win10 的 Acrylic 走未公开 API，拖动窗口时会被系统临时撤掉
/// 再恢复，表现为窗口内容落后于边框（拖拽漂移），所以 Win10 改用更轻的 Blur。
pub fn enable(window: &WebviewWindow) -> Result<(), String> {
    if is_windows_11() {
        window_vibrancy::apply_acrylic(window, None)
            .map_err(|e| format!("当前系统不支持毛玻璃透出：{e}"))
    } else {
        window_vibrancy::apply_blur(window, Some((250, 240, 225, 120)))
            .map_err(|e| format!("当前系统不支持背景模糊：{e}"))
    }
}

/// 关闭毛玻璃：按系统版本清对应的效果。
pub fn disable(window: &WebviewWindow) -> Result<(), String> {
    if is_windows_11() {
        window_vibrancy::clear_acrylic(window).map_err(|e| format!("关闭毛玻璃失败：{e}"))
    } else {
        window_vibrancy::clear_blur(window).map_err(|e| format!("关闭背景模糊失败：{e}"))
    }
}

/// 按配置应用；返回 Err 表示系统不支持，调用方应把配置回滚为关闭。
pub fn apply(window: &WebviewWindow, enabled: bool) -> Result<(), String> {
    if enabled {
        enable(window)
    } else {
        disable(window)
    }
}
