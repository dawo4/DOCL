//! 下载源测速：并发请求各源的小文件，给出延迟与可用性，
//! 方便用户判断当前网络适合官方源还是镜像源。

use futures::future::join_all;
use serde::Serialize;
use std::time::{Duration, Instant};
use tauri::State;

use crate::AppState;

#[derive(Debug, Clone, Serialize)]
pub struct SourceProbe {
    pub id: String,
    pub name: String,
    /// mojang = 游戏文件与加载器；mod = CurseForge / Modrinth 接口
    pub group: String,
    pub url: String,
    pub ok: bool,
    pub latency_ms: u64,
    pub bytes: usize,
    /// 失败原因，成功时为空
    pub error: String,
}

/// (id, 名称, 分组, 探测地址)
const SOURCES: &[(&str, &str, &str, &str)] = &[
    (
        "mojang",
        "Mojang 官方",
        "mojang",
        "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json",
    ),
    (
        "bmclapi",
        "BMCLAPI 镜像",
        "mojang",
        "https://bmclapi2.bangbang93.com/mc/game/version_manifest_v2.json",
    ),
    (
        "forge_maven",
        "Forge Maven 官方",
        "mojang",
        "https://maven.minecraftforge.net/net/minecraftforge/forge/maven-metadata.xml",
    ),
    (
        "bmclapi_maven",
        "BMCLAPI Maven",
        "mojang",
        "https://bmclapi2.bangbang93.com/maven/net/minecraftforge/forge/maven-metadata.xml",
    ),
    (
        "mcim_curseforge",
        "MCIM · CurseForge",
        "mod",
        "https://mod.mcimirror.top/curseforge/v1/mods/238222",
    ),
    (
        "curse_tools",
        "curse.tools · CurseForge",
        "mod",
        "https://api.curse.tools/v1/cf/mods/238222",
    ),
    (
        "modrinth",
        "Modrinth 官方",
        "mod",
        "https://api.modrinth.com/v2/project/fabric-api",
    ),
    (
        "mcim_modrinth",
        "MCIM · Modrinth",
        "mod",
        "https://mod.mcimirror.top/modrinth/v2/project/fabric-api",
    ),
    (
        "cursemaven",
        "cursemaven 直链解析",
        "mod",
        "https://cursemaven.com/test/238222/2724420",
    ),
];

async fn probe_one(client: &reqwest::Client, source: &(&str, &str, &str, &str)) -> SourceProbe {
    let (id, name, group, url) = *source;
    let started = Instant::now();
    let result = client
        .get(url)
        // 只取头部一小段，测的是「能否连上 + 首包延迟」，不浪费流量
        .header(reqwest::header::RANGE, "bytes=0-65535")
        .timeout(Duration::from_secs(15))
        .send()
        .await;
    let elapsed = started.elapsed().as_millis() as u64;
    let mut probe = SourceProbe {
        id: id.to_string(),
        name: name.to_string(),
        group: group.to_string(),
        url: url.to_string(),
        ok: false,
        latency_ms: elapsed,
        bytes: 0,
        error: String::new(),
    };
    match result {
        Ok(response) => {
            let status = response.status();
            if status.is_success() {
                match response.bytes().await {
                    Ok(body) => {
                        probe.ok = true;
                        probe.bytes = body.len();
                    }
                    Err(err) => probe.error = format!("读取响应失败：{err}"),
                }
            } else {
                probe.error = format!("HTTP {status}");
            }
        }
        Err(err) => probe.error = err.to_string(),
    }
    probe
}

#[tauri::command]
pub async fn probe_download_sources(state: State<'_, AppState>) -> Result<Vec<SourceProbe>, String> {
    let client = state.client.clone();
    let results = join_all(SOURCES.iter().map(|source| probe_one(&client, source))).await;
    Ok(results)
}
