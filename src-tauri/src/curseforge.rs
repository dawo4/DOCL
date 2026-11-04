use crate::config::Config;
use crate::download::{self, FileTask, ProgressState};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tauri::AppHandle;

const BASE: &str = "https://api.curse.tools/v1/cf";
/// 备用 API 源（MCIM 国内镜像，PCL 也用它），海外代理被墙或拒绝时兜底
const MIRROR_BASE: &str = "https://mod.mcimirror.top/curseforge/v1";

/// 依次尝试两个 API 源，返回第一个成功的结果。
/// 默认镜像优先：curse.tools 在国内经常完全连不上（实测 HTTP 000，等满超时才失败）。
async fn get_json_api<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    path: &str,
    prefer_mirror: bool,
) -> Result<T> {
    let bases: [&str; 2] = if prefer_mirror {
        [MIRROR_BASE, BASE]
    } else {
        [BASE, MIRROR_BASE]
    };
    let mut last_error: Option<anyhow::Error> = None;
    for base in bases {
        let url = format!("{base}{path}");
        // 单个源最多等 12 秒，避免被墙的源拖满整轮超时
        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(12),
            download::get_json::<T>(client, &url, false),
        )
        .await;
        match outcome {
            Ok(Ok(value)) => return Ok(value),
            Ok(Err(err)) => {
                #[cfg(debug_assertions)]
                eprintln!("[curseforge] {base} 请求 {path} 失败: {err}");
                last_error = Some(err);
            }
            Err(_) => {
                #[cfg(debug_assertions)]
                eprintln!("[curseforge] {base} 请求 {path} 超时");
                last_error = Some(anyhow!("CurseForge 接口超时"));
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    Err(last_error.unwrap_or_else(|| anyhow!("CurseForge 接口不可用")))
}

/// 把基于 BASE 拼好的完整 URL 转成相对路径，交给多源请求。
fn relative_path(full: &str) -> String {
    full.trim_start_matches(BASE).to_string()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CfLogo {
    #[serde(default)]
    thumbnail_url: String,
    #[serde(default)]
    url: String,
}

#[derive(Debug, Clone, Deserialize)]
struct CfAuthor {
    #[serde(default)]
    name: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct CfHash {
    #[serde(default)]
    value: String,
    /// 1 = SHA1，2 = MD5
    #[serde(default)]
    algo: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CfFile {
    id: i64,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    file_name: String,
    #[serde(default)]
    file_date: String,
    #[serde(default)]
    file_length: u64,
    #[serde(default)]
    hashes: Option<Vec<CfHash>>,
    #[serde(default)]
    download_url: Option<String>,
    #[serde(default)]
    game_versions: Vec<String>,
    #[serde(default)]
    release_type: i32,
    /// 该文件本身是服务端整合包
    #[serde(default)]
    is_server_pack: bool,
    /// 对应服务端包的文件 ID（客户端包指向配套服务端）
    #[serde(default)]
    server_pack_file_id: Option<i64>,
    #[serde(default)]
    dependencies: Vec<CfDependency>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CfDependency {
    #[serde(default)]
    mod_id: i64,
    /// 3 = 必需前置，2 = 可选，1 = 内置
    #[serde(default)]
    relation_type: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CfCategory {
    #[serde(default)]
    name: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CfLinks {
    #[serde(default)]
    website_url: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CfMod {
    id: i64,
    #[serde(default)]
    name: String,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    download_count: u64,
    #[serde(default)]
    class_id: i64,
    #[serde(default)]
    date_modified: String,
    #[serde(default)]
    categories: Vec<CfCategory>,
    logo: Option<CfLogo>,
    #[serde(default)]
    authors: Vec<CfAuthor>,
    #[serde(default)]
    links: Option<CfLinks>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CfPagination {
    #[serde(default)]
    total_count: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct CfSearchResponse {
    #[serde(default)]
    data: Vec<CfMod>,
    pagination: Option<CfPagination>,
}

#[derive(Debug, Clone, Deserialize)]
struct CfFilesResponse {
    #[serde(default)]
    data: Vec<CfFile>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CfProject {
    pub source: String,
    pub id: String,
    pub name: String,
    pub slug: String,
    /// 官方项目网页地址，前端「转到官网」按钮直接打开
    pub website_url: String,
    pub summary: String,
    pub download_count: u64,
    pub class_id: i64,
    pub logo_url: String,
    pub author: String,
    pub updated: String,
    pub categories: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CfSearchResult {
    pub items: Vec<CfProject>,
    pub total: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct CfCategoryListResponse {
    #[serde(default)]
    data: Vec<CfCategoryRaw>,
}

#[derive(Debug, Clone, Deserialize)]
struct CfCategoryRaw {
    #[serde(default)]
    id: i64,
    #[serde(default)]
    name: String,
}

/// CurseForge 分类（类型筛选下拉用）。
#[derive(Debug, Clone, Serialize)]
pub struct CfCategoryView {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CfFileView {
    pub id: String,
    pub name: String,
    pub file_name: String,
    pub file_date: String,
    pub size: u64,
    pub download_url: Option<String>,
    pub game_versions: Vec<String>,
    pub release_type: i32,
    /// 该文件本身是服务端整合包
    pub is_server_pack: bool,
    /// 对应服务端包的文件 ID，可据此一键下载服务端
    pub server_pack_file_id: Option<String>,
    /// 必需前置的项目 ID，供界面显示「N 项前置」
    pub dependencies: Vec<String>,
}

fn loader_id(loader: &str) -> Option<&'static str> {
    match loader {
        "forge" => Some("1"),
        "fabric" => Some("4"),
        "quilt" => Some("5"),
        "neoforge" => Some("6"),
        _ => None,
    }
}

pub async fn search(
    client: &reqwest::Client,
    class_id: i64,
    query: &str,
    game_version: Option<&str>,
    loader: Option<&str>,
    category_id: Option<i64>,
    index: u32,
    page_size: u32,
    sort_field: u32,
) -> Result<CfSearchResult> {
    let mut url = reqwest::Url::parse(&format!("{BASE}/mods/search"))?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs
            .append_pair("gameId", "432")
            .append_pair("classId", &class_id.to_string())
            .append_pair("index", &index.to_string())
            .append_pair("pageSize", &page_size.to_string())
            .append_pair("sortField", &sort_field.to_string())
            .append_pair("sortOrder", "desc");
        if !query.trim().is_empty() {
            pairs.append_pair("searchFilter", query.trim());
        }
        if let Some(version) = game_version {
            if !version.is_empty() {
                pairs.append_pair("gameVersion", version);
            }
        }
        if let Some(loader) = loader.and_then(loader_id) {
            pairs.append_pair("modLoaderType", loader);
        }
        if let Some(category_id) = category_id {
            pairs.append_pair("categoryId", &category_id.to_string());
        }
    }

    let response: CfSearchResponse =
        get_json_api(client, &relative_path(url.as_str()), true).await?;
    let items = response.data.into_iter().map(to_project).collect();
    Ok(CfSearchResult {
        items,
        total: response
            .pagination
            .map(|p| p.total_count)
            .unwrap_or(0),
    })
}

fn to_project(project: CfMod) -> CfProject {
    let links = project.links.unwrap_or_default();
    // CF 模组的网页路径是 mc-mods，mods 路径会 404；其余分类由 websiteUrl 官方地址兜底
    let website_url = if links.website_url.is_empty() {
        format!("https://www.curseforge.com/minecraft/mc-mods/{}", project.slug)
    } else {
        links.website_url
    };
    CfProject {
        source: "curseforge".into(),
        id: project.id.to_string(),
        name: project.name,
        slug: project.slug,
        website_url,
        summary: project.summary,
        download_count: project.download_count,
        class_id: project.class_id,
        logo_url: project
            .logo
            .map(|logo| {
                if logo.thumbnail_url.is_empty() {
                    logo.url
                } else {
                    logo.thumbnail_url
                }
            })
            .unwrap_or_default(),
        author: project
            .authors
            .first()
            .map(|author| author.name.clone())
            .unwrap_or_default(),
        updated: project.date_modified,
        categories: project
            .categories
            .into_iter()
            .map(|category| category.name)
            .filter(|name| !name.is_empty())
            .collect(),
    }
}

/// 按项目 ID 取详情，用于下载页展示前置资源。
pub async fn projects(client: &reqwest::Client, ids: &[i64]) -> Vec<CfProject> {
    #[derive(Deserialize)]
    struct CfModResponse {
        data: CfMod,
    }
    let mut result = Vec::new();
    for id in ids.iter().take(10) {
        let path = format!("/mods/{id}");
        if let Ok(response) = get_json_api::<CfModResponse>(client, &path, true).await {
            result.push(to_project(response.data));
        }
    }
    result
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CfGameVersion {
    #[serde(default)]
    version_string: String,
    #[serde(default)]
    date_modified: String,
}

#[derive(Debug, Clone, Deserialize)]
struct CfGameVersionsResponse {
    #[serde(default)]
    data: Vec<CfGameVersion>,
}

/// 拉取某资源大类下的分类列表，供类型筛选下拉使用。
pub async fn categories(client: &reqwest::Client, class_id: i64) -> Result<Vec<CfCategoryView>> {
    let mut url = reqwest::Url::parse(&format!("{BASE}/categories"))?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs
            .append_pair("gameId", "432")
            .append_pair("classId", &class_id.to_string());
    }
    let response: CfCategoryListResponse =
        get_json_api(client, &relative_path(url.as_str()), true).await?;
    let mut list: Vec<CfCategoryView> = response
        .data
        .into_iter()
        .map(|raw| CfCategoryView {
            id: raw.id,
            name: raw.name,
        })
        .collect();
    list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(list)
}

/// 供版本下拉框使用的正式版列表，按发布时间倒序。
pub async fn game_versions(client: &reqwest::Client) -> Result<Vec<String>> {
    let response: CfGameVersionsResponse =
        get_json_api(client, "/minecraft/version", true).await?;
    let mut list: Vec<CfGameVersion> = response
        .data
        .into_iter()
        .filter(|entry| {
            let value = &entry.version_string;
            value.contains('.')
                && value
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == '.')
        })
        .collect();
    list.sort_by(|a, b| b.date_modified.cmp(&a.date_modified));
    Ok(list
        .into_iter()
        .map(|entry| entry.version_string)
        .take(60)
        .collect())
}

pub async fn files(
    client: &reqwest::Client,
    mod_id: i64,
    game_version: Option<&str>,
    loader: Option<&str>,
) -> Result<Vec<CfFileView>> {
    let mut url = reqwest::Url::parse(&format!("{BASE}/mods/{mod_id}/files"))?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("pageSize", "50");
        if let Some(version) = game_version {
            if !version.is_empty() {
                pairs.append_pair("gameVersion", version);
            }
        }
        if let Some(loader) = loader.and_then(loader_id) {
            pairs.append_pair("modLoaderType", loader);
        }
    }
    let response: CfFilesResponse = get_json_api(client, &relative_path(url.as_str()), true).await?;
    let mut files: Vec<CfFileView> = response
        .data
        .into_iter()
        .map(|file| CfFileView {
            id: file.id.to_string(),
            name: if file.display_name.is_empty() {
                file.file_name.clone()
            } else {
                file.display_name
            },
            file_name: file.file_name,
            file_date: file.file_date,
            size: file.file_length,
            download_url: file.download_url,
            game_versions: file.game_versions,
            release_type: file.release_type,
            is_server_pack: file.is_server_pack,
            server_pack_file_id: file
                .server_pack_file_id
                .filter(|value| *value > 0)
                .map(|value| value.to_string()),
            dependencies: file
                .dependencies
                .into_iter()
                .filter(|dep| dep.relation_type == 3 && dep.mod_id > 0)
                .map(|dep| dep.mod_id.to_string())
                .collect(),
        })
        .collect();
    files.sort_by(|a, b| b.file_date.cmp(&a.file_date));
    files.truncate(25);
    Ok(files)
}

#[derive(Debug, Clone, Serialize)]
pub struct FileDownload {
    pub url: String,
    pub name: String,
    pub sha1: Option<String>,
    pub size: u64,
}

/// 解析 cursemaven 的 /test 调试输出：里面有服务端解析出来的直链与文件大小。
/// 形如：
/// ```
/// Found: https://edge.forgecdn.net/files/7125/700/indestructible-20.13.0.jar
///      Content-Length: 264932
/// ```
fn parse_cursemaven(text: &str) -> Option<(String, u64)> {
    let mut url: Option<String> = None;
    let mut size = 0u64;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Found:") {
            let value = rest.trim();
            if value.starts_with("https://") || value.starts_with("http://") {
                url = Some(value.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("Content-Length:") {
            size = rest.trim().parse().unwrap_or(0);
        }
    }
    url.map(|value| (value, size))
}

/// 只接受已知 CDN 主机：第三方解析结果不能随便把启动器引到任意地址下载
fn trusted_cdn_url(url: &str) -> bool {
    const PREFIXES: [&str; 5] = [
        "https://edge.forgecdn.net/",
        "https://mediafilez.forgecdn.net/",
        "https://media.forgecdn.net/",
        "https://mod.mcimirror.top/",
        "https://cdn.modrinth.com/",
    ];
    PREFIXES.iter().any(|prefix| url.starts_with(prefix))
}

/// 兜底解析文件直链：cursemaven 服务端带 API Key 调官方接口，
/// 能拿到 MCIM 未收录、国内又直连不了的文件的真实文件名。
/// 这是国内可用网络下拿到「文件名」的最后一条路（文件本体走 MCIM CDN 代理）。
async fn cursemaven_download(
    client: &reqwest::Client,
    mod_id: i64,
    file_id: i64,
) -> Option<FileDownload> {
    let url = format!("https://cursemaven.com/test/{mod_id}/{file_id}");
    let response = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let text = response.text().await.ok()?;
    let (download_url, size) = parse_cursemaven(&text)?;
    if !trusted_cdn_url(&download_url) {
        return None;
    }
    let path = download_url.split('?').next().unwrap_or("").trim_end_matches('/');
    let name = path.rsplit('/').next().unwrap_or_default().to_string();
    if name.is_empty() {
        return None;
    }
    Some(FileDownload {
        url: download_url,
        name,
        // cursemaven 不提供哈希；大小来自 CDN 的 Content-Length，拿不到时为 0（不校验）
        sha1: None,
        size,
    })
}

/// 读取文件详情并取出下载直链。
/// 403/404 属于服务端明确拒绝，原地重试没有意义，像 PCL 一样直接换源：
/// 先试配置偏好的源，失败再换另一个；每个源内部仍有一次重试。
pub async fn file_download(
    client: &reqwest::Client,
    mod_id: i64,
    file_id: i64,
    prefer_mirror: bool,
) -> Result<FileDownload> {
    #[derive(Deserialize)]
    struct CfFileResponse {
        data: CfFile,
    }

    let path = format!("/mods/{mod_id}/files/{file_id}");
    let file = match get_json_api::<CfFileResponse>(client, &path, prefer_mirror).await {
        Ok(response) => response.data,
        Err(err) => {
            // 两个信息源都拿不到时，试 cursemaven 的服务端解析（只影响极少数文件）
            if let Some(file) = cursemaven_download(client, mod_id, file_id).await {
                #[cfg(debug_assertions)]
                eprintln!("[curseforge] {mod_id}/{file_id} 通过 cursemaven 兜底解析成功");
                return Ok(file);
            }
            return Err(err);
        }
    };

    let file_name = file.file_name.clone();
    // 作者关闭第三方分发时接口会把 downloadUrl 置空，但 CDN 仍按 fileId 分段可直达。
    // 官方 CDN 对这情况返回 404，镜像能取到，交给下载层的镜像轮换处理。
    let download_url = match file.download_url.filter(|url| !url.trim().is_empty()) {
        Some(url) => url,
        None => {
            if file_name.trim().is_empty() {
                return Err(anyhow!("作者未开放第三方下载，且接口未返回文件名"));
            }
            format!(
                "https://edge.forgecdn.net/files/{}/{}/{}",
                file.id / 1000,
                file.id % 1000,
                file_name
            )
        }
    };
    let name = if file_name.is_empty() {
        format!("{mod_id}-{file_id}.jar")
    } else {
        file_name
    };
    let sha1 = file
        .hashes
        .as_deref()
        .unwrap_or_default()
        .iter()
        .find(|hash| hash.algo == 1 && !hash.value.is_empty())
        .map(|hash| hash.value.clone());
    Ok(FileDownload {
        url: download_url,
        name,
        sha1,
        size: file.file_length,
    })
}

#[derive(Debug, Serialize)]
pub struct FileLink {
    pub url: String,
    pub file_name: String,
}

/// 前端一键下载服务端包时使用：按项目 + 文件 ID 取直链。
pub async fn file_link(
    client: &reqwest::Client,
    mod_id: &str,
    file_id: &str,
    prefer_mirror: bool,
) -> Result<FileLink> {
    let mod_id: i64 = mod_id.parse().context("项目 ID 无效")?;
    let file_id: i64 = file_id.parse().context("文件 ID 无效")?;
    let download = file_download(client, mod_id, file_id, prefer_mirror).await?;
    Ok(FileLink {
        url: download.url,
        file_name: download.name,
    })
}

#[derive(Debug, Serialize)]
pub struct InstallOutcome {
    pub path: String,
    pub dependencies: crate::moddeps::DependencyReport,
}

pub async fn install_file(
    app: &AppHandle,
    client: &reqwest::Client,
    cfg: &Config,
    instance_id: &str,
    kind: &str,
    url: &str,
    file_name: &str,
    progress: Arc<ProgressState>,
) -> Result<InstallOutcome> {
    let name = Path::new(file_name)
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "download.bin".to_string());
    let dir = crate::instance::content_dir(cfg, instance_id, kind)?;
    let dest = dir.join(&name);
    let task = FileTask {
        url: url.to_string(),
        dest: dest.clone(),
        sha1: None,
        size: 0,
        label: name,
    };
    download::run_tasks(app, client, vec![task], 1, cfg.use_mirror(), progress.clone()).await?;

    // 模组装好后顺手补齐它声明的必需前置，装不上的留给前端提示
    let mut dependencies = crate::moddeps::DependencyReport::default();
    if matches!(kind, "mods" | "mod") {
        progress.set_message("检查模组前置…");
        download::emit_now(app, &progress);
        dependencies = crate::moddeps::install_missing(
            app,
            client,
            cfg,
            instance_id,
            &dest,
            progress.clone(),
        )
        .await;
    }

    // 前置检查结束后必须改写进度文案，否则面板会一直停在「检查模组前置…」
    let done = if !dependencies.installed.is_empty() {
        format!("安装完成，已补齐前置 {}", dependencies.installed.join("、"))
    } else if !dependencies.unresolved.is_empty() {
        format!("安装完成，缺少前置 {}", dependencies.unresolved.join("、"))
    } else {
        "安装完成".to_string()
    };
    progress.set_message(done);

    Ok(InstallOutcome {
        path: dest.to_string_lossy().to_string(),
        dependencies,
    })
}

#[cfg(test)]
mod tests {
    use super::parse_cursemaven;

    /// cursemaven 的调试输出里同时有直链和 Content-Length
    #[test]
    fn parses_cursemaven_output() {
        let text = "Raw URL: /test/915201/7125700\n\
ProjectDescriptor: 915201\n\
FileIds: 7125700\n\
Found: https://edge.forgecdn.net/files/7125/700/indestructible-20.13.0.jar\n\
 - GET Response: 200\n\
     Content-Type: application/java-archive\n\
     Content-Length: 264932\n";
        let (url, size) = parse_cursemaven(text).unwrap();
        assert_eq!(url, "https://edge.forgecdn.net/files/7125/700/indestructible-20.13.0.jar");
        assert_eq!(size, 264932);
    }

    /// 解析失败（例如接口返回错误信息）时不能返回假地址
    #[test]
    fn rejects_cursemaven_without_url() {
        assert!(parse_cursemaven("Response: 404\nNot found").is_none());
    }

    /// 第三方解析结果只允许落到已知 CDN 主机
    #[test]
    fn only_trusts_known_cdn_hosts() {
        assert!(super::trusted_cdn_url(
            "https://edge.forgecdn.net/files/7125/700/indestructible-20.13.0.jar"
        ));
        assert!(super::trusted_cdn_url(
            "https://mod.mcimirror.top/files/7125/700/indestructible-20.13.0.jar"
        ));
        assert!(!super::trusted_cdn_url("https://evil.example.com/steal.jar"));
    }
}
