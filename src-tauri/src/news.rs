use crate::config;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const BASE: &str = "https://launchercontent.mojang.com";
const NEWS_URL: &str = "https://launchercontent.mojang.com/v2/news.json";
const PATCH_URL: &str = "https://launchercontent.mojang.com/v2/javaPatchNotes.json";
const WIKI_API: &str = "https://zh.minecraft.wiki/api.php";
const WIKI_BASE: &str = "https://zh.minecraft.wiki/wiki/";
const CACHE_TTL: u64 = 30 * 60;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct NewsItem {
    pub id: String,
    pub title: String,
    pub category: String,
    pub tag: String,
    pub date: String,
    pub summary: String,
    pub image_url: Option<String>,
    pub url: Option<String>,
    pub source: String,
    pub version: Option<String>,
    pub kind: Option<String>,
    pub detail_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NewsFeed {
    pub items: Vec<NewsItem>,
    pub updated_at: u64,
    pub stale: bool,
    pub errors: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct CacheFile {
    fetched_at: u64,
    #[serde(default)]
    items: Vec<NewsItem>,
}

#[derive(Deserialize)]
struct ImageRef {
    #[serde(default)]
    url: String,
}

#[derive(Deserialize)]
struct OfficialFeed {
    #[serde(default)]
    entries: Vec<OfficialEntry>,
}

#[derive(Deserialize)]
struct OfficialEntry {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    tag: Option<String>,
    #[serde(default)]
    date: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(rename = "newsType", default)]
    news_type: Vec<String>,
    #[serde(rename = "readMoreLink", default)]
    read_more_link: Option<String>,
    #[serde(rename = "playPageImage", default)]
    play_page_image: Option<ImageRef>,
    #[serde(rename = "newsPageImage", default)]
    news_page_image: Option<ImageRef>,
}

#[derive(Deserialize)]
struct PatchFeed {
    #[serde(default)]
    entries: Vec<PatchEntry>,
}

#[derive(Deserialize)]
struct PatchEntry {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    version: String,
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    date: String,
    #[serde(default)]
    image: Option<ImageRef>,
    #[serde(rename = "contentPath", default)]
    content_path: String,
}

#[derive(Deserialize)]
struct PatchDetail {
    #[serde(default)]
    body: String,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0)
}

fn cache_path() -> PathBuf {
    // v2：条目改为中文标题/摘要并带 Wiki 链接，弃用旧缓存避免 30 分钟内一直显示英文
    config::data_dir().join("news_cache_v2.json")
}

fn read_cache() -> Option<CacheFile> {
    std::fs::read(cache_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

fn write_cache(items: &[NewsItem]) {
    let _ = std::fs::create_dir_all(config::data_dir());
    let payload = serde_json::json!({
        "fetched_at": now_secs(),
        "items": items,
    });
    if let Ok(bytes) = serde_json::to_vec(&payload) {
        let _ = std::fs::write(cache_path(), bytes);
    }
}

fn image_url(image: Option<ImageRef>) -> Option<String> {
    image.and_then(|image| {
        let url = image.url.trim();
        if url.is_empty() {
            None
        } else if url.starts_with("http://") || url.starts_with("https://") {
            Some(url.to_string())
        } else if url.starts_with('/') {
            Some(format!("{BASE}{url}"))
        } else {
            Some(format!("{BASE}/{url}"))
        }
    })
}

fn truncate_text(input: &str, max: usize) -> String {
    let cleaned = input.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = cleaned.chars();
    let mut out: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        out.push('…');
    }
    out
}

/// minecraft.net 文章链接切到中文站：已翻译的文章直接是中文，未翻译的至少带中文界面。
fn zh_article_url(link: &str) -> Option<String> {
    let rest = link.split("minecraft.net").nth(1)?;
    let rest = rest.strip_prefix("/en-us").unwrap_or(rest);
    let path = rest.split('?').next()?.trim_end_matches('/');
    if !path.starts_with("/article/") || path.len() <= "/article/".len() {
        return None;
    }
    Some(format!("https://www.minecraft.net/zh-hans{path}"))
}

fn is_java_news(entry: &OfficialEntry) -> bool {
    entry
        .news_type
        .iter()
        .any(|item| item.eq_ignore_ascii_case("java"))
        || entry.category.to_lowercase().contains("java")
}

async fn fetch_official(client: &reqwest::Client) -> Result<Vec<NewsItem>> {
    let feed: OfficialFeed = client
        .get(NEWS_URL)
        .timeout(Duration::from_secs(8))
        .send()
        .await
        .context("请求官方新闻失败")?
        .error_for_status()?
        .json()
        .await
        .context("解析官方新闻失败")?;

    let mut items: Vec<NewsItem> = feed
        .entries
        .into_iter()
        .filter(is_java_news)
        .map(|entry| NewsItem {
            id: entry.id,
            title: entry.title.trim().to_string(),
            category: "Java Edition".into(),
            tag: entry
                .tag
                .map(|tag| tag.trim().to_string())
                .filter(|tag| !tag.is_empty())
                .unwrap_or_else(|| "NEWS".into()),
            date: entry.date.trim().to_string(),
            summary: truncate_text(entry.text.as_deref().unwrap_or_default(), 160),
            image_url: image_url(entry.play_page_image.or(entry.news_page_image)),
            url: entry
                .read_more_link
                .filter(|link| !link.trim().is_empty())
                .map(|link| zh_article_url(&link).unwrap_or(link)),
            source: "official".into(),
            version: None,
            kind: None,
            detail_path: None,
        })
        .collect();
    items.sort_by(|a, b| b.date.cmp(&a.date));
    items.truncate(12);
    Ok(items)
}

fn patch_tag(kind: &str) -> String {
    match kind {
        "snapshot" => "快照".into(),
        "release" => "正式版".into(),
        "pre-release" => "预发布".into(),
        "release-candidate" => "候选版".into(),
        other if !other.is_empty() => other.to_string(),
        _ => "更新".into(),
    }
}

/// 把内部版本号转成可读形式：26.4-snapshot-5 → 26.4 快照 5。
fn pretty_version(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some((base, number)) = trimmed.split_once("-snapshot-") {
        format!("{base} 快照 {number}")
    } else {
        trimmed.to_string()
    }
}

/// 版本条目的中文摘要模板，替代英文 shortText。
fn zh_patch_summary(kind: &str, pretty: &str) -> String {
    if pretty.is_empty() {
        return String::new();
    }
    match kind {
        "snapshot" => format!("{pretty}：测试新特性并修复一批问题，可能损坏存档，游玩前请备份"),
        "pre-release" => format!("预发布版 {pretty}：快照阶段收尾，集中修复遗留问题"),
        "release-candidate" => format!("候选版 {pretty}：若无重大问题将在近期转为正式版"),
        "release" => format!("正式版 {pretty} 发布，汇总此前快照的全部新特性与修复"),
        _ => format!("Minecraft Java 版 {pretty} 已更新"),
    }
}

/// 非 ASCII 路径段转百分号编码（中文 Wiki 页面标题需要）。
fn percent_encode_path(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 3);
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// 批量查询中文 Wiki 是否存在版本页，返回实际存在的标题集合。
async fn wiki_pages_exist(client: &reqwest::Client, titles: &[String]) -> HashSet<String> {
    static EMPTY: std::sync::LazyLock<HashSet<String>> = std::sync::LazyLock::new(HashSet::new);
    if titles.is_empty() {
        return HashSet::new();
    }

    #[derive(Deserialize)]
    struct WikiQuery {
        #[serde(default)]
        query: Option<WikiQueryInner>,
    }
    #[derive(Deserialize)]
    struct WikiQueryInner {
        #[serde(default)]
        pages: Vec<WikiPage>,
    }
    #[derive(Deserialize)]
    struct WikiPage {
        #[serde(default)]
        title: String,
        #[serde(default)]
        missing: bool,
        #[serde(default)]
        invalid: bool,
    }

    let joined = titles.join("|");
    let response = client
        .get(WIKI_API)
        .query(&[
            ("action", "query"),
            ("titles", joined.as_str()),
            ("redirects", "1"),
            ("format", "json"),
            ("formatversion", "2"),
        ])
        .timeout(Duration::from_secs(6))
        .send()
        .await;
    let response = match response {
        Ok(value) => value,
        Err(_) => return EMPTY.clone(),
    };
    let parsed: WikiQuery = match response.json().await {
        Ok(value) => value,
        Err(_) => return EMPTY.clone(),
    };
    parsed
        .query
        .map(|inner| {
            inner
                .pages
                .into_iter()
                .filter(|page| !page.missing && !page.invalid)
                .map(|page| page.title)
                .collect()
        })
        .unwrap_or_default()
}

async fn fetch_patch_notes(client: &reqwest::Client) -> Result<Vec<NewsItem>> {
    let feed: PatchFeed = client
        .get(PATCH_URL)
        .timeout(Duration::from_secs(8))
        .send()
        .await
        .context("请求版本更新日志失败")?
        .error_for_status()?
        .json()
        .await
        .context("解析版本更新日志失败")?;

    let mut items: Vec<NewsItem> = feed
        .entries
        .into_iter()
        .map(|entry| {
            let version = entry.version.trim().to_string();
            let pretty = pretty_version(&version);
            let title = if pretty.is_empty() {
                entry.title.trim().to_string()
            } else {
                format!("Minecraft Java 版 {pretty}")
            };
            NewsItem {
                id: entry.id,
                title,
                category: "Java 版本".into(),
                tag: patch_tag(&entry.kind),
                date: entry.date.chars().take(10).collect(),
                summary: zh_patch_summary(&entry.kind, &pretty),
                image_url: image_url(entry.image),
                url: None,
                source: "patch".into(),
                version: if version.is_empty() {
                    None
                } else {
                    Some(version)
                },
                kind: if entry.kind.trim().is_empty() {
                    None
                } else {
                    Some(entry.kind)
                },
                detail_path: if entry.content_path.trim().is_empty() {
                    None
                } else {
                    Some(entry.content_path)
                },
            }
        })
        .collect();
    items.sort_by(|a, b| b.date.cmp(&a.date));
    items.truncate(8);

    // 给存在中文 Wiki 页面的版本挂上汉化介绍链接（单次批量查询，失败不影响列表）
    let wiki_titles: Vec<String> = items
        .iter()
        .filter_map(|item| item.version.as_ref().map(|version| format!("Java版{version}")))
        .collect();
    let existing = wiki_pages_exist(client, &wiki_titles).await;
    for item in items.iter_mut() {
        if let Some(version) = item.version.as_ref() {
            let title = format!("Java版{version}");
            if existing.contains(&title) {
                item.url = Some(format!("{WIKI_BASE}{}", percent_encode_path(&title)));
            }
        }
    }
    Ok(items)
}

/// 获取新闻聚合，优先命中缓存；失败时回退到旧缓存。
pub async fn feed(client: &reqwest::Client, force: bool) -> NewsFeed {
    let now = now_secs();
    if !force {
        if let Some(cache) = read_cache() {
            if now.saturating_sub(cache.fetched_at) < CACHE_TTL {
                return NewsFeed {
                    items: cache.items,
                    updated_at: cache.fetched_at,
                    stale: false,
                    errors: Vec::new(),
                };
            }
        }
    }

    let (official, patch) = futures::join!(fetch_official(client), fetch_patch_notes(client));
    let mut items = Vec::new();
    let mut errors = Vec::new();
    match official {
        Ok(mut list) => items.append(&mut list),
        Err(err) => errors.push(format!("官方新闻：{err}")),
    }
    match patch {
        Ok(mut list) => items.append(&mut list),
        Err(err) => errors.push(format!("版本日志：{err}")),
    }

    if items.is_empty() {
        if let Some(cache) = read_cache() {
            return NewsFeed {
                items: cache.items,
                updated_at: cache.fetched_at,
                stale: true,
                errors,
            };
        }
        return NewsFeed {
            items: Vec::new(),
            updated_at: now,
            stale: true,
            errors,
        };
    }

    items.sort_by(|a, b| b.date.cmp(&a.date));
    let mut seen = HashSet::new();
    items.retain(|item| seen.insert(format!("{}::{}", item.source, item.id)));
    write_cache(&items);

    NewsFeed {
        items,
        updated_at: now,
        stale: false,
        errors,
    }
}

/// 把更新日志 HTML 转成可读纯文本，避免前端直接渲染 HTML。
fn html_to_text(html: &str) -> String {
    let mut text = html.to_string();
    for tag in ["</h1>", "</h2>", "</h3>", "</p>", "</li>", "</ul>", "</ol>"] {
        text = text.replace(tag, "\n");
    }
    for tag in ["<br>", "<br/>", "<br />"] {
        text = text.replace(tag, "\n");
    }
    text = text.replace("<li>", "\n• ");

    let mut stripped = String::with_capacity(text.len());
    let mut in_tag = false;
    for ch in text.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => stripped.push(ch),
            _ => {}
        }
    }

    let decoded = stripped
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'");

    let mut lines: Vec<String> = Vec::new();
    for line in decoded.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if lines.last().map(|last| !last.is_empty()).unwrap_or(false) {
                lines.push(String::new());
            }
        } else {
            lines.push(trimmed.to_string());
        }
    }
    while lines.last().map(|line| line.is_empty()).unwrap_or(false) {
        lines.pop();
    }
    lines.join("\n")
}

/// 把 Wiki 纯文本里的 == 标题 == 转成更易读的样式并压缩空行。
fn prettify_wiki_extract(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        let rendered = if let Some(inner) =
            trimmed.strip_prefix("== ").and_then(|rest| rest.strip_suffix(" =="))
        {
            format!("【{inner}】")
        } else if let Some(inner) =
            trimmed.strip_prefix("=== ").and_then(|rest| rest.strip_suffix(" ==="))
        {
            format!("· {inner}")
        } else {
            trimmed
                .trim_start_matches(|c| c == ';' || c == '*' || c == ' ')
                .to_string()
        };
        if rendered.is_empty() {
            if lines.last().map(|last| !last.is_empty()).unwrap_or(false) {
                lines.push(String::new());
            }
        } else {
            lines.push(rendered);
        }
    }
    while lines.last().map(|line| line.is_empty()).unwrap_or(false) {
        lines.pop();
    }
    lines.join("\n")
}

/// 拉取中文 Wiki 版本页的纯文本汉化内容（TextExtracts 接口），失败自动重试一次。
async fn wiki_extract(client: &reqwest::Client, title: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct WikiExtractQuery {
        #[serde(default)]
        query: Option<WikiExtractInner>,
    }
    #[derive(Deserialize)]
    struct WikiExtractInner {
        #[serde(default)]
        pages: Vec<WikiExtractPage>,
    }
    #[derive(Deserialize)]
    struct WikiExtractPage {
        #[serde(default)]
        extract: Option<String>,
    }

    for attempt in 0..2 {
        let response = client
            .get(WIKI_API)
            .query(&[
                ("action", "query"),
                ("prop", "extracts"),
                ("explaintext", "1"),
                ("exsectionformat", "wiki"),
                ("titles", title),
                ("format", "json"),
                ("formatversion", "2"),
            ])
            .timeout(Duration::from_secs(12))
            .send()
            .await;
        let result = match response {
            Ok(value) => match value.json::<WikiExtractQuery>().await {
                Ok(parsed) => {
                    let extract = parsed
                        .query
                        .and_then(|inner| inner.pages.into_iter().find_map(|page| page.extract))
                        .filter(|text| !text.trim().is_empty());
                    if extract.is_none() {
                        #[cfg(debug_assertions)]
                        eprintln!(
                            "[news] 中文 Wiki 未返回正文（第 {} 次）: {title}",
                            attempt + 1
                        );
                    }
                    extract
                }
                Err(err) => {
                    #[cfg(debug_assertions)]
                    eprintln!("[news] 中文 Wiki 响应解析失败（第 {} 次）: {err}", attempt + 1);
                    #[cfg(not(debug_assertions))]
                    let _ = &err;
                    None
                }
            },
            Err(err) => {
                #[cfg(debug_assertions)]
                eprintln!("[news] 中文 Wiki 请求失败（第 {} 次）: {err}", attempt + 1);
                #[cfg(not(debug_assertions))]
                let _ = &err;
                None
            }
        };
        if result.is_some() {
            return result;
        }
        if attempt == 0 {
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
    }
    None
}

/// 拉取单条版本更新日志正文；若提供 Wiki 标题则把中文 Wiki 汉化介绍放在正文前面。
/// 中英文两路请求并行执行，任一路失败都不影响另一路。
pub async fn article(
    client: &reqwest::Client,
    path: &str,
    wiki_title: Option<&str>,
) -> Result<String> {
    let relative = path.trim();
    if !relative.starts_with("javaPatchNotes/") || relative.contains("..") {
        bail!("无效的更新日志路径");
    }

    let english = async {
        let detail: PatchDetail = client
            .get(format!("{BASE}/v2/{relative}"))
            .timeout(Duration::from_secs(8))
            .send()
            .await
            .context("请求更新日志失败")?
            .error_for_status()?
            .json()
            .await
            .context("解析更新日志失败")?;
        let text = html_to_text(&detail.body);
        if text.trim().is_empty() {
            bail!("更新日志正文为空");
        }
        Ok(text)
    };

    let wiki = async {
        let title = wiki_title.map(str::trim).filter(|title| !title.is_empty());
        match title {
            Some(title) => wiki_extract(client, title).await,
            None => None,
        }
    };

    let (english, wiki) = tokio::join!(english, wiki);
    let english = english?;

    let mut combined = String::new();
    if let Some(wiki) = wiki {
        let pretty = prettify_wiki_extract(&wiki);
        if !pretty.is_empty() {
            combined.push_str(&pretty);
            combined.push_str("\n\n——— 以下为 Mojang 官方英文原文 ———\n\n");
        }
    }
    combined.push_str(&english);
    Ok(combined)
}
