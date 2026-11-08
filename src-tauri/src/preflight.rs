use crate::config::Config;
use crate::{install, instance, java, moddeps, system};
use serde::Serialize;
use std::path::Path;

/// 启动前自检发现问题。分成阻断与警告两档：
/// 阻断项不修好基本必然启动失败，警告项可以继续但很可能在游戏里出问题。
#[derive(Debug, Clone, Serialize)]
pub struct PreflightIssue {
    pub id: String,
    pub title: String,
    pub detail: String,
    /// 是否阻止启动
    pub blocking: bool,
    /// 是否能用现有的一键修复处理
    pub fixable: bool,
}

#[derive(Debug, Serialize)]
pub struct PreflightReport {
    pub version_id: String,
    pub issues: Vec<PreflightIssue>,
}

fn push(
    issues: &mut Vec<PreflightIssue>,
    id: &str,
    title: &str,
    detail: String,
    blocking: bool,
    fixable: bool,
) {
    issues.push(PreflightIssue {
        id: id.into(),
        title: title.into(),
        detail,
        blocking,
        fixable,
    });
}

/// 文件存在且不是 0 字节才算可用；下载中断最容易留下空文件。
fn file_ready(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|meta| meta.len() > 0)
        .unwrap_or(false)
}

/// 游戏所在磁盘的剩余空间。拿不到就跳过，不因为一次系统调用失败就误报。
#[cfg(windows)]
fn free_space_bytes(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);
    let mut free: u64 = 0;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(free)
}

#[cfg(not(windows))]
fn free_space_bytes(_path: &Path) -> Option<u64> {
    None
}

/// 供修复流程判断某个 Java 是不是 32 位；读不到 PE 头时按 64 位处理。
pub(crate) fn is_32bit_java(exe: &Path) -> bool {
    pe_machine(exe) == Some(0x014c)
}

/// 直接读 PE 头判断 java.exe 的位数，比跑一次 java 进程快得多。
/// 返回目标机器的 Machine 字段：0x014c = x86，0x8664 = x64。
fn pe_machine(exe: &Path) -> Option<u16> {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = std::fs::File::open(exe).ok()?;
    let mut dos_header = [0u8; 64];
    file.read_exact(&mut dos_header).ok()?;
    if &dos_header[0..2] != b"MZ" {
        return None;
    }
    let pe_offset = u32::from_le_bytes([
        dos_header[0x3c],
        dos_header[0x3d],
        dos_header[0x3e],
        dos_header[0x3f],
    ]) as u64;
    file.seek(SeekFrom::Start(pe_offset)).ok()?;
    let mut pe_header = [0u8; 6];
    file.read_exact(&mut pe_header).ok()?;
    if &pe_header[0..4] != b"PE\0\0" {
        return None;
    }
    Some(u16::from_le_bytes([pe_header[4], pe_header[5]]))
}

/// 内存配置是否合理。返回 (是否阻断, 说明)。
fn memory_verdict(memory_mb: u32, total_mb: u64) -> Option<(bool, String)> {
    if total_mb == 0 {
        return None;
    }
    if memory_mb as u64 > total_mb {
        return Some((
            true,
            format!("当前分配 {memory_mb} MB，已超过物理内存总量 {total_mb} MB，JVM 会直接分配失败。"),
        ));
    }
    if memory_mb as u64 > total_mb * 3 / 4 {
        return Some((
            false,
            format!("当前分配 {memory_mb} MB，接近物理内存总量 {total_mb} MB，建议至少留四分之一给系统。"),
        ));
    }
    None
}

/// 启动前自检。只读检查，不修改任何文件，可以随时重复调用。
pub fn check(cfg: &Config, version_id: &str) -> PreflightReport {
    let mut issues: Vec<PreflightIssue> = Vec::new();
    let game_dir = instance::game_dir(cfg, version_id);

    // 1. 版本清单：缺失或损坏时后面所有检查都没有意义，直接返回
    let merged = match install::resolve_detail(cfg, version_id) {
        Ok(value) => value,
        Err(error) => {
            push(
                &mut issues,
                "version-json",
                "版本清单无法读取",
                format!("{error}。一键修复会尝试重新拉取清单与启动文件。"),
                true,
                true,
            );
            return PreflightReport {
                version_id: version_id.into(),
                issues,
            };
        }
    };

    // 2. 客户端 jar
    if let Some(jar_id) = merged.jar_id.as_ref() {
        let jar = cfg
            .game_root()
            .join("versions")
            .join(jar_id)
            .join(format!("{jar_id}.jar"));
        if !file_ready(&jar) {
            push(
                &mut issues,
                "client-jar",
                "客户端 jar 缺失或为空",
                format!("{}。通常是下载中断或被清理软件删掉。", jar.display()),
                true,
                true,
            );
        }
    }

    // 3. 启动库
    let mut missing = 0usize;
    let mut first_missing: Option<std::path::PathBuf> = None;
    for path in install::library_file_paths(cfg, &merged.detail) {
        if file_ready(&path) {
            continue;
        }
        missing += 1;
        if first_missing.is_none() {
            first_missing = Some(path);
        }
    }
    if missing > 0 {
        let sample = first_missing
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        push(
            &mut issues,
            "libraries",
            "启动库文件缺失",
            format!("{missing} 个库文件缺失或为空，例如 {sample}。"),
            true,
            true,
        );
    }

    // 4. Mod 重复：同一个 mod id 装了多份，加载器会直接报 DuplicateModsFoundException
    let duplicates = moddeps::duplicate_mods(&game_dir.join("mods"));
    if !duplicates.is_empty() {
        let names = duplicates
            .iter()
            .take(3)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>()
            .join("、");
        let suffix = if duplicates.len() > 3 { " 等" } else { "" };
        push(
            &mut issues,
            "duplicate-mods",
            "同一个 Mod 装了多份",
            format!("{names}{suffix}。一键修复会只保留一份。"),
            false,
            true,
        );
    }

    // 5. 磁盘空间
    if let Some(free) = free_space_bytes(&game_dir) {
        const MIN_FREE: u64 = 2 * 1024 * 1024 * 1024;
        if free < MIN_FREE {
            push(
                &mut issues,
                "disk-space",
                "磁盘剩余空间不足",
                format!(
                    "游戏目录所在磁盘只剩 {:.1} GB，建议清理到 2 GB 以上再启动。",
                    free as f64 / 1024.0 / 1024.0 / 1024.0
                ),
                false,
                false,
            );
        }
    }

    // 6. 内存分配
    let memory = system::memory_info();
    if !cfg.memory_auto {
        if let Some((blocking, detail)) = memory_verdict(cfg.memory_mb, memory.total_mb) {
            push(
                &mut issues,
                "memory-size",
                if blocking {
                    "分配的内存超过物理内存"
                } else {
                    "分配的内存偏高"
                },
                detail,
                blocking,
                false,
            );
        }
        if cfg.memory_mb < 1024 {
            push(
                &mut issues,
                "memory-low",
                "分配的内存偏低",
                format!(
                    "当前只分配了 {} MB，大型整合包很容易内存不足。",
                    cfg.memory_mb
                ),
                false,
                false,
            );
        }
    }

    // 7. 32 位 Java：最多只能用约 1.5 GB 堆内存，大整合包必然撑不住
    if let Some(raw) = cfg
        .java_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if let Some(exe) = java::resolve_java_path(raw) {
            if pe_machine(&exe) == Some(0x014c) {
                let blocking = cfg.memory_mb > 1536;
                push(
                    &mut issues,
                    "java-32bit",
                    "正在使用 32 位 Java",
                    if blocking {
                        format!(
                            "{} 是 32 位 Java，最多只能用约 1.5 GB 内存，当前分配了 {} MB，启动很可能失败。请换成 64 位 Java。",
                            exe.display(),
                            cfg.memory_mb
                        )
                    } else {
                        format!(
                            "{} 是 32 位 Java，最多只能用约 1.5 GB 内存，大型整合包容易内存不足。",
                            exe.display()
                        )
                    },
                    blocking,
                    false,
                );
            }
        }
    }

    // 8. 路径过长
    let path_len = game_dir.to_string_lossy().chars().count();
    if path_len > 200 {
        push(
            &mut issues,
            "long-path",
            "游戏目录路径过长",
            format!("当前路径 {path_len} 个字符，接近 Windows 上限，建议把游戏目录移到更短的路径。"),
            false,
            false,
        );
    }

    PreflightReport {
        version_id: version_id.into(),
        issues,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_version_json_blocks_launch() {
        let cfg = Config::default();
        let report = check(&cfg, "__docl_missing_version__");
        assert!(report.issues.iter().any(|issue| issue.blocking));
        assert_eq!(report.issues[0].id, "version-json");
        assert!(report.issues[0].fixable);
    }

    #[test]
    fn memory_over_total_blocks() {
        let (blocking, _) = memory_verdict(8192, 4096).expect("应当给出结论");
        assert!(blocking);
    }

    #[test]
    fn memory_near_total_only_warns() {
        let (blocking, _) = memory_verdict(3800, 4096).expect("应当给出结论");
        assert!(!blocking);
    }

    #[test]
    fn memory_within_budget_is_fine() {
        assert!(memory_verdict(4096, 16384).is_none());
        // 读不到物理内存时不做判断，避免误报
        assert!(memory_verdict(65536, 0).is_none());
    }
}
