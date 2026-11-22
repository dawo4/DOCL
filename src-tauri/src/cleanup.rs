use crate::config::{self, Config};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// 清理结果：删了多少文件、释放了多少空间。
#[derive(Debug, Default, Serialize)]
pub struct JunkReport {
    pub files: usize,
    pub freed: u64,
    /// 删除失败的项数，通常是文件正被游戏或下载任务占用
    pub failed: usize,
    /// 失败示例，最多保留几条，方便排查
    pub failed_samples: Vec<String>,
}

/// 缓存类目录只清超过这个时间的，避免刚下过的文件又要重下。
const CACHE_KEEP_DAYS: u64 = 7;
const MAX_FAILED_SAMPLES: usize = 5;

/// 清理启动器缓存与各版本的游戏日志、崩溃报告。
/// 只删可再生成的文件，不碰存档、Mod、配置与修复备份。
pub fn clean(cfg: &Config) -> JunkReport {
    let mut report = JunkReport::default();
    let data = config::data_dir();
    let keep = Duration::from_secs(CACHE_KEEP_DAYS * 24 * 3600);

    // 纯临时目录，里面没有需要保留的内容
    for name in ["tmp", "temp"] {
        remove_children(&data.join(name), &mut report);
    }
    // 缓存与导出记录只清旧的：刚下载的更新包、刚导出的报告还在用
    for name in [
        "download-cache",
        "installers",
        "updates",
        "crash-exports",
        "crash-feedback",
    ] {
        remove_older_files(&data.join(name), keep, &mut report);
    }

    // 所有配置过的游戏目录都要清，不只当前目录
    let mut roots: Vec<PathBuf> = cfg
        .game_dirs
        .iter()
        .map(|entry| PathBuf::from(&entry.path))
        .collect();
    let active = cfg.game_root();
    if !roots.iter().any(|path| path == &active) {
        roots.push(active);
    }
    for root in roots {
        clean_root(&root, &mut report);
    }

    report
}

fn clean_root(root: &Path, report: &mut JunkReport) {
    // 未启用版本隔离时日志在游戏目录根部
    clean_instance_dir(root, report);
    // 启用版本隔离时每个版本有自己的 logs / crash-reports
    let Ok(entries) = std::fs::read_dir(root.join("versions")) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().map(|item| item.is_dir()).unwrap_or(false) {
            clean_instance_dir(&entry.path(), report);
        }
    }
}

fn clean_instance_dir(dir: &Path, report: &mut JunkReport) {
    // 整个目录都可以重新生成，直接删掉
    for name in ["crash-reports", "debug"] {
        remove_path(&dir.join(name), report);
    }
    // logs 里保留 latest.log：崩溃分析还要读它
    if let Ok(entries) = std::fs::read_dir(dir.join("logs")) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if path.file_name().and_then(|name| name.to_str()) == Some("latest.log") {
                continue;
            }
            let is_log = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| {
                    let lower = name.to_ascii_lowercase();
                    lower.ends_with(".log") || lower.ends_with(".log.gz") || lower.ends_with(".gz")
                })
                .unwrap_or(false);
            if is_log {
                delete_file(&path, report);
            }
        }
    }
    // JVM 崩溃时写在游戏目录根部的 hs_err_pid*.log / *.mdmp
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if name.starts_with("hs_err_pid") && entry.path().is_file() {
                delete_file(&entry.path(), report);
            }
        }
    }
}

fn remove_children(dir: &Path, report: &mut JunkReport) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        remove_path(&entry.path(), report);
    }
}

fn remove_path(path: &Path, report: &mut JunkReport) {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return;
    };
    if meta.file_type().is_dir() {
        let mut files = 0usize;
        let mut bytes = 0u64;
        tally(path, &mut files, &mut bytes);
        match std::fs::remove_dir_all(path) {
            Ok(()) => {
                report.files += files;
                report.freed += bytes;
            }
            Err(err) => push_failure(report, path, &err),
        }
    } else if meta.is_file() {
        let len = meta.len();
        match std::fs::remove_file(path) {
            Ok(()) => {
                report.files += 1;
                report.freed += len;
            }
            Err(err) => push_failure(report, path, &err),
        }
    }
}

fn delete_file(path: &Path, report: &mut JunkReport) {
    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };
    match std::fs::remove_file(path) {
        Ok(()) => {
            report.files += 1;
            report.freed += meta.len();
        }
        Err(err) => push_failure(report, path, &err),
    }
}

/// 递归统计目录里的文件数与总大小，用于删除前计算释放空间。
fn tally(dir: &Path, files: &mut usize, bytes: &mut u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            tally(&path, files, bytes);
            continue;
        }
        if let Ok(meta) = std::fs::metadata(&path) {
            *files += 1;
            *bytes += meta.len();
        }
    }
}

fn remove_older_files(dir: &Path, keep: Duration, report: &mut JunkReport) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let Some(cutoff) = SystemTime::now().checked_sub(keep) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // updates/pending 这类嵌套目录也要能清
            remove_older_files(&path, keep, report);
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|meta| meta.modified()) else {
            continue;
        };
        if modified < cutoff {
            delete_file(&path, report);
        }
    }
}

fn push_failure(report: &mut JunkReport, path: &Path, err: &std::io::Error) {
    report.failed += 1;
    if report.failed_samples.len() < MAX_FAILED_SAMPLES {
        report
            .failed_samples
            .push(format!("{}：{err}", path.display()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("docl-clean-{tag}-{stamp}"))
    }

    #[test]
    fn removes_logs_but_keeps_latest_log() {
        let dir = temp_dir("logs");
        let logs = dir.join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(logs.join("latest.log"), b"current").unwrap();
        std::fs::write(logs.join("2024-01-01-1.log.gz"), b"old").unwrap();
        std::fs::write(logs.join("readme.txt"), b"keep").unwrap();

        let mut report = JunkReport::default();
        clean_instance_dir(&dir, &mut report);

        assert_eq!(report.files, 1);
        assert!(logs.join("latest.log").is_file());
        assert!(!logs.join("2024-01-01-1.log.gz").exists());
        assert!(logs.join("readme.txt").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn removes_crash_reports_and_hs_err_files() {
        let dir = temp_dir("crash");
        let reports = dir.join("crash-reports");
        std::fs::create_dir_all(&reports).unwrap();
        std::fs::write(reports.join("crash-1.txt"), b"crash").unwrap();
        std::fs::write(dir.join("hs_err_pid123.log"), b"jvm crash").unwrap();
        std::fs::write(dir.join("saves.zip"), b"keep").unwrap();

        let mut report = JunkReport::default();
        clean_instance_dir(&dir, &mut report);

        assert_eq!(report.files, 2);
        assert!(!reports.exists());
        assert!(!dir.join("hs_err_pid123.log").exists());
        assert!(dir.join("saves.zip").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_cache_files_expire_but_fresh_files_stay() {
        let dir = temp_dir("cache");
        std::fs::create_dir_all(&dir).unwrap();
        let fresh = dir.join("fresh.bin");
        std::fs::write(&fresh, b"abc").unwrap();

        let mut report = JunkReport::default();
        remove_older_files(&dir, Duration::from_secs(3600), &mut report);
        assert_eq!(report.files, 0);
        assert!(fresh.is_file());

        // 0 秒保留期等价于全部过期
        remove_older_files(&dir, Duration::from_secs(0), &mut report);
        assert_eq!(report.files, 1);
        assert!(!fresh.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
