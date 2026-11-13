use serde::Serialize;
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::ProcessStatus::EmptyWorkingSet;
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_SET_QUOTA,
};

#[derive(Debug, Clone, Copy, Serialize)]
pub struct MemoryInfo {
    pub total_mb: u64,
    pub available_mb: u64,
    /// 按当前占用情况推荐的自动分配值
    pub recommended_mb: u32,
}

/// 读取物理内存总量与可用量（MB）。
pub fn memory_info() -> MemoryInfo {
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    if ok == 0 {
        return MemoryInfo {
            total_mb: 0,
            available_mb: 0,
            recommended_mb: 4096,
        };
    }
    let info = MemoryInfo {
        total_mb: status.ullTotalPhys / 1024 / 1024,
        available_mb: status.ullAvailPhys / 1024 / 1024,
        recommended_mb: 0,
    };
    MemoryInfo {
        recommended_mb: recommended_memory_mb(info.total_mb, info.available_mb),
        ..info
    }
}

/// 参考 PCL 的自动内存分配：
/// 以「当前可用物理内存」为基准取约 3/4——浏览器、启动器等占用变化会实时反映在推荐值里；
/// 上限为总内存的 3/4 且至少给系统留 2GB（总内存不足 8GB 时留 1GB），推荐值封顶 12GB；
/// 下限 1GB，按 512MB 对齐。
pub fn recommended_memory_mb(total_mb: u64, available_mb: u64) -> u32 {
    let total = if total_mb > 0 { total_mb } else { return 4096 };
    let available = available_mb.min(total);
    let mut target = available * 3 / 4;
    let leave: u64 = if total >= 8192 { 2048 } else { 1024 };
    let cap = total
        .saturating_sub(leave)
        .min(total * 3 / 4)
        .min(12288)
        .max(1024);
    target = target.clamp(1024, cap);
    // 512MB 对齐（向下取整）
    let aligned = (target / 512) * 512;
    aligned.max(1024) as u32
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct MemoryRelease {
    /// 优化前后系统可用内存的差值（MB），可能为 0
    pub freed_mb: u64,
    /// 成功清理工作集的进程数
    pub processes: u32,
}

/// 参考 PCL 百宝箱的内存优化：遍历所有能打开的进程，把闲置的工作集还给系统。
/// 只清工作集、不结束进程，拿不到权限的进程直接跳过。
pub fn optimize_memory() -> MemoryRelease {
    let before = memory_info().available_mb;
    let mut processes = 0u32;

    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return MemoryRelease {
                freed_mb: 0,
                processes: 0,
            };
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snapshot, &mut entry);
        while ok != 0 {
            let handle = OpenProcess(
                PROCESS_QUERY_INFORMATION | PROCESS_SET_QUOTA,
                0,
                entry.th32ProcessID,
            );
            if !handle.is_null() {
                if EmptyWorkingSet(handle) != 0 {
                    processes += 1;
                }
                CloseHandle(handle);
            }
            ok = Process32NextW(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
    }

    // 页面回收由系统异步完成，稍等一下再读可用内存
    std::thread::sleep(std::time::Duration::from_millis(250));
    let after = memory_info().available_mb;
    MemoryRelease {
        freed_mb: after.saturating_sub(before),
        processes,
    }
}

#[cfg(test)]
mod tests {
    use super::recommended_memory_mb;

    #[test]
    fn auto_memory_follows_available() {
        // 8GB 机器：空闲推荐 6GB，占用上来后明显下降
        assert_eq!(recommended_memory_mb(8192, 8192), 6144);
        assert_eq!(recommended_memory_mb(8192, 4096), 3072);
        assert_eq!(recommended_memory_mb(8192, 1024), 1024);
        // 16GB 机器空闲 14GB：10500 → 对齐 10752
        assert_eq!(recommended_memory_mb(16384, 14336), 10752);
        // 32GB 空闲时封顶 12GB，不会给出离谱的大堆
        assert_eq!(recommended_memory_mb(32768, 32768), 12288);
        // 2GB 小机器至少给系统留 1GB
        assert_eq!(recommended_memory_mb(2048, 2048), 1024);
        // 512MB 对齐（向下取整）
        assert_eq!(recommended_memory_mb(16384, 7000), 5120);
        // 读不到内存时的兜底
        assert_eq!(recommended_memory_mb(0, 0), 4096);
    }
}
