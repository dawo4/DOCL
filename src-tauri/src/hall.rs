use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

const MAX_ROOMS: usize = 100;
const FETCH_TIMEOUT: u64 = 12;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HallRoom {
    /// 陶瓦房间邀请码
    pub room: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub motd: String,
    #[serde(default)]
    pub players: u32,
    #[serde(default)]
    pub max_players: u32,
    #[serde(default)]
    pub version: String,
    /// 房主使用的加载器（可空，大厅源未提供时不校验）
    #[serde(default)]
    pub loader: String,
    /// 房间需要的 Mod 文件或 mod id（可空）
    #[serde(default)]
    pub mods: Vec<String>,
    /// Unix 秒；用于显示“几分钟前”
    #[serde(default)]
    pub updated: u64,
}

/// 从用户配置的大厅源拉取房间列表。
/// 支持 `{"rooms": [...]}` 或直接返回数组两种格式。
pub async fn fetch(client: &reqwest::Client, url: &str) -> Result<Vec<HallRoom>> {
    let url = url.trim();
    if url.is_empty() {
        return Ok(Vec::new());
    }
    let parsed = reqwest::Url::parse(url).context("大厅地址格式无效")?;
    if !matches!(parsed.scheme(), "http" | "https") {
        bail!("大厅地址必须以 http:// 或 https:// 开头");
    }

    let response = tokio::time::timeout(Duration::from_secs(FETCH_TIMEOUT), async {
        client
            .get(parsed)
            .send()
            .await
            .context("连接大厅源失败")
    })
    .await
    .map_err(|_| anyhow::anyhow!("大厅源响应超时"))??;

    if !response.status().is_success() {
        bail!("大厅源返回 HTTP {}", response.status());
    }
    let bytes = response.bytes().await.context("读取大厅数据失败")?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).context("大厅数据不是有效 JSON")?;

    let list = value
        .get("rooms")
        .and_then(|item| item.as_array())
        .or_else(|| value.as_array())
        .cloned()
        .unwrap_or_default();

    let mut rooms: Vec<HallRoom> = list
        .into_iter()
        .filter_map(|item| serde_json::from_value::<HallRoom>(item).ok())
        .filter(|room| !room.room.trim().is_empty())
        .collect();
    rooms.sort_by(|a, b| b.updated.cmp(&a.updated));
    rooms.truncate(MAX_ROOMS);
    Ok(rooms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_room_list_shapes() {
        let wrapped = r#"{"rooms":[{"room":"U/AAAA","name":"A","players":2}]}"#;
        let value: serde_json::Value = serde_json::from_str(wrapped).unwrap();
        let list = value
            .get("rooms")
            .and_then(|item| item.as_array())
            .cloned()
            .unwrap();
        let rooms: Vec<HallRoom> = list
            .into_iter()
            .filter_map(|item| serde_json::from_value(item).ok())
            .collect();
        assert_eq!(rooms.len(), 1);
        assert_eq!(rooms[0].room, "U/AAAA");
        assert_eq!(rooms[0].players, 2);

        let bare = r#"[{"room":"U/BBBB"}]"#;
        let value: serde_json::Value = serde_json::from_str(bare).unwrap();
        assert!(value.as_array().is_some());
    }
}
