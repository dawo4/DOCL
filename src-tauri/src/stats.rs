use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// 单个版本的游玩累计
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct VersionStat {
    #[serde(default)]
    pub launches: u64,
    #[serde(default)]
    pub total_seconds: u64,
    /// 首次/最近游玩时间（Unix 秒）
    #[serde(default)]
    pub first_played: Option<u64>,
    #[serde(default)]
    pub last_played: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub version: String,
    pub started: u64,
    pub seconds: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PlayStats {
    #[serde(default)]
    pub versions: BTreeMap<String, VersionStat>,
    /// 最近几次游玩，仅用于展示（含「最近 7 天」柱图），最多保留 200 条
    #[serde(default)]
    pub sessions: Vec<Session>,
}

const MAX_SESSIONS: usize = 200;

fn store_path() -> PathBuf {
    crate::config::data_dir().join("play-stats.json")
}

fn lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn read() -> PlayStats {
    std::fs::read(store_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn write(stats: &PlayStats) -> std::io::Result<()> {
    let path = store_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_vec_pretty(stats).map_err(std::io::Error::other)?;
    // 先写临时文件再替换，避免写到一半被关机/结束进程时把整份历史截断
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    std::fs::rename(&tmp, &path)
}

/// 一次游戏退出后记录时长；读写失败静默忽略，不能影响启动流程本身。
pub fn record(version_id: &str, started: u64, seconds: u64) {
    let Ok(_guard) = lock().lock() else {
        return;
    };
    let mut stats = read();
    let entry = stats.versions.entry(version_id.to_string()).or_default();
    entry.launches = entry.launches.saturating_add(1);
    entry.total_seconds = entry.total_seconds.saturating_add(seconds);
    entry.last_played = Some(started.saturating_add(seconds));
    if entry.first_played.is_none() {
        entry.first_played = Some(started);
    }
    stats.sessions.insert(
        0,
        Session {
            version: version_id.to_string(),
            started,
            seconds,
        },
    );
    stats.sessions.truncate(MAX_SESSIONS);
    let _ = write(&stats);
}

pub fn summary() -> PlayStats {
    let Ok(_guard) = lock().lock() else {
        return PlayStats::default();
    };
    read()
}

#[cfg(test)]
mod tests {
    use super::PlayStats;

    #[test]
    fn defaults_when_file_missing() {
        let stats = PlayStats::default();
        assert!(stats.versions.is_empty());
        assert!(stats.sessions.is_empty());
    }

    #[test]
    fn parses_partial_json() {
        let stats: PlayStats = serde_json::from_str("{}").unwrap();
        assert!(stats.versions.is_empty());
    }
}
