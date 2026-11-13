//! LittleSkin 同步：OAuth2 授权码流 + 衣柜读取 + 把皮肤应用到站点角色。
//!
//! 站点接口要求 Bearer 令牌，Yggdrasil 的 accessToken 不能直接复用，
//! 所以这里单独走 LittleSkin 的 OAuth2（授权码 + 本地回环回调）。

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::auth::dpapi;

pub const SITE: &str = "https://littleskin.cn";
/// 本地回调端口：在 LittleSkin 应用里登记的回调必须与这里完全一致
const PORT: u16 = 28326;
pub const REDIRECT_URI: &str = "http://127.0.0.1:28326/callback";
const SCOPE: &str = "User.Read Closet.Read Player.ReadWrite";
const WAIT_SECONDS: u64 = 300;

fn store_path() -> PathBuf {
    crate::config::data_dir().join("littleskin.json")
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn enc(value: &str) -> Result<String> {
    if value.is_empty() {
        return Ok(String::new());
    }
    Ok(STANDARD.encode(dpapi::protect(value.as_bytes())?))
}

fn dec(value: &str) -> Result<String> {
    if value.is_empty() {
        return Ok(String::new());
    }
    let blob = STANDARD.decode(value)?;
    Ok(String::from_utf8(dpapi::unprotect(&blob)?)?)
}

#[derive(Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    client_secret: String,
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    expires_at: u64,
    #[serde(default)]
    uid: u64,
    #[serde(default)]
    name: String,
}

fn load() -> Store {
    std::fs::read(store_path())
        .ok()
        .and_then(|raw| serde_json::from_slice::<Store>(&raw).ok())
        .unwrap_or_default()
}

fn save(store: &Store) -> Result<()> {
    let path = store_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, serde_json::to_vec_pretty(store)?)?;
    Ok(())
}

#[derive(Serialize)]
pub struct StatusView {
    pub configured: bool,
    pub authorized: bool,
    pub name: String,
    pub uid: u64,
    /// 供界面提示用户在 LittleSkin 应用里填写的回调地址
    pub redirect_uri: String,
    pub scope: String,
}

fn status_now() -> StatusView {
    let store = load();
    let configured = !store.client_id.is_empty() && !store.client_secret.is_empty();
    let authorized = !store.access_token.is_empty() || !store.refresh_token.is_empty();
    StatusView {
        configured,
        authorized: authorized && configured,
        name: store.name,
        uid: store.uid,
        redirect_uri: REDIRECT_URI.to_string(),
        scope: SCOPE.to_string(),
    }
}

pub fn status() -> StatusView {
    status_now()
}

/// 保存应用凭据；换了 Client ID 就作废旧令牌，避免把令牌用到别的应用上。
pub fn set_app(client_id: String, client_secret: String) -> Result<StatusView> {
    let client_id = client_id.trim().to_string();
    let client_secret = client_secret.trim().to_string();
    if client_id.is_empty() || client_secret.is_empty() {
        bail!("Client ID 和 Client Secret 都不能为空");
    }
    let mut store = load();
    if store.client_id != client_id {
        store.access_token.clear();
        store.refresh_token.clear();
        store.expires_at = 0;
        store.name.clear();
        store.uid = 0;
    }
    store.client_id = client_id;
    store.client_secret = enc(&client_secret)?;
    save(&store)?;
    Ok(status_now())
}

pub fn logout() -> Result<()> {
    let mut store = load();
    store.access_token.clear();
    store.refresh_token.clear();
    store.expires_at = 0;
    store.name.clear();
    store.uid = 0;
    save(&store)
}

#[derive(Deserialize)]
struct TokenResp {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: u64,
}

async fn token_request(
    client: &reqwest::Client,
    params: &[(&str, &str)],
) -> Result<TokenResp> {
    let resp = client
        .post(format!("{SITE}/oauth/token"))
        .header("Accept", "application/json")
        .form(params)
        .send()
        .await
        .context("无法连接 LittleSkin，请检查网络")?;
    let status = resp.status();
    let text = resp.text().await?;
    if !status.is_success() {
        let message = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|value| {
                value
                    .get("error_description")
                    .or_else(|| value.get("message"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| format!("授权失败（HTTP {status}）"));
        bail!(message);
    }
    serde_json::from_str(&text).context("LittleSkin 返回了无法识别的授权响应")
}

async fn api_value(resp: reqwest::Response) -> Result<serde_json::Value> {
    let status = resp.status();
    let text = resp.text().await?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|_| serde_json::Value::String(text.clone()));
    if !status.is_success() {
        let message = value
            .get("message")
            .and_then(|v| v.as_str())
            .or_else(|| value.get("error_description").and_then(|v| v.as_str()))
            .map(str::to_string)
            .unwrap_or_else(|| format!("LittleSkin 请求失败（HTTP {status}）"));
        bail!(message);
    }
    if let Some(code) = value.get("code").and_then(|v| v.as_i64()) {
        if code != 0 {
            let message = value
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("LittleSkin 拒绝了这次请求");
            bail!("{message}");
        }
    }
    Ok(value)
}

/// 取可用令牌，过期就用 refresh_token 换新的（LittleSkin 会轮换 refresh_token）。
async fn access_token(client: &reqwest::Client) -> Result<String> {
    let mut store = load();
    if store.client_id.is_empty() || store.client_secret.is_empty() {
        bail!("还没有配置 LittleSkin 应用");
    }
    let token = dec(&store.access_token)?;
    if !token.is_empty() && store.expires_at > now() + 30 {
        return Ok(token);
    }
    let refresh = dec(&store.refresh_token)?;
    if refresh.is_empty() {
        bail!("LittleSkin 授权已失效，请重新授权");
    }
    let secret = dec(&store.client_secret)?;
    let resp = token_request(
        client,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh.as_str()),
            ("client_id", store.client_id.as_str()),
            ("client_secret", secret.as_str()),
        ],
    )
    .await
    .map_err(|err| anyhow::anyhow!("LittleSkin 授权已失效，请重新授权（{err}）"))?;
    store.access_token = enc(&resp.access_token)?;
    if let Some(next) = resp.refresh_token {
        store.refresh_token = enc(&next)?;
    }
    store.expires_at = now() + resp.expires_in;
    save(&store)?;
    Ok(resp.access_token)
}

fn url_encode(input: &str) -> String {
    let mut out = String::new();
    for byte in input.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn url_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                let decoded = input
                    .get(index + 1..index + 3)
                    .and_then(|hex| u8::from_str_radix(hex, 16).ok());
                if let Some(byte) = decoded {
                    out.push(byte);
                    index += 3;
                    continue;
                }
                out.push(bytes[index]);
                index += 1;
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        if url_decode(name) == key {
            Some(url_decode(value))
        } else {
            None
        }
    })
}

fn page_html(title: &str, body: &str) -> String {
    let html = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>{title}</title>\
         <body style=\"margin:0;height:100vh;display:grid;place-items:center;background:#f7f0e2;\
         color:#2b2118;font-family:system-ui,sans-serif\">\
         <div style=\"text-align:center\"><h2 style=\"margin:0 0 10px\">{title}</h2>\
         <p style=\"color:#7a6a55\">{body}</p></div></body>"
    );
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    )
}

/// 等待浏览器跳回本地回调，返回授权代码。
async fn wait_callback(listener: &TcpListener, expected_state: &str) -> Result<String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(WAIT_SECONDS);
    loop {
        let accepted = tokio::time::timeout_at(deadline, listener.accept()).await;
        let (mut socket, _) = match accepted {
            Ok(Ok(pair)) => pair,
            Ok(Err(err)) => return Err(err.into()),
            Err(_) => bail!("等待授权超时，请重新发起授权"),
        };
        let mut buffer = [0u8; 4096];
        let read = socket.read(&mut buffer).await.unwrap_or(0);
        let request = String::from_utf8_lossy(&buffer[..read]).to_string();
        let first = request.lines().next().unwrap_or("");
        let path = first.split_whitespace().nth(1).unwrap_or("");
        let (route, query) = path.split_once('?').unwrap_or((path, ""));
        if route != "/callback" {
            let _ = socket
                .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                .await;
            continue;
        }
        let state = query_param(query, "state").unwrap_or_default();
        let code = query_param(query, "code");
        let error = query_param(query, "error_description");
        let html = if state != expected_state {
            page_html("授权失败", "状态校验未通过，请回到 DOCL 重新发起授权。")
        } else if code.is_none() {
            page_html(
                "授权未完成",
                error.as_deref().unwrap_or("没有拿到授权代码，请重新授权。"),
            )
        } else {
            page_html("授权成功", "可以关闭这个页面，回到 DOCL 继续操作。")
        };
        let _ = socket.write_all(html.as_bytes()).await;
        let _ = socket.shutdown().await;
        if state != expected_state {
            bail!("授权状态校验失败，请重试");
        }
        match code {
            Some(code) => return Ok(code),
            None => bail!(error.unwrap_or_else(|| "没有拿到授权代码".into())),
        }
    }
}

fn open_browser(url: &str) -> Result<()> {
    std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn()
        .context("无法打开浏览器")?;
    Ok(())
}

async fn fetch_user(client: &reqwest::Client, token: &str) -> Result<(u64, String)> {
    let resp = client
        .get(format!("{SITE}/api/user"))
        .header("Accept", "application/json")
        .bearer_auth(token)
        .send()
        .await?;
    let value = api_value(resp).await?;
    // 兼容裸对象与 { code, data } 两种返回
    let user = value.get("data").unwrap_or(&value);
    let uid = user
        .get("uid")
        .or_else(|| user.get("id"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let name = user
        .get("nickname")
        .or_else(|| user.get("username"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok((uid, name))
}

/// 拉起浏览器完成授权，成功后立即拉一次用户信息并存盘。
pub async fn authorize(client: &reqwest::Client) -> Result<StatusView> {
    let mut store = load();
    if store.client_id.is_empty() || store.client_secret.is_empty() {
        bail!("请先填写 LittleSkin 应用的 Client ID 和 Client Secret");
    }
    let listener = TcpListener::bind(("127.0.0.1", PORT))
        .await
        .with_context(|| format!("本地回调端口 {PORT} 被占用，请先关闭占用它的程序"))?;
    let state = uuid::Uuid::new_v4().simple().to_string();
    let url = format!(
        "{SITE}/oauth/authorize?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}",
        url_encode(&store.client_id),
        url_encode(REDIRECT_URI),
        url_encode(SCOPE),
        state
    );
    open_browser(&url)?;
    let code = wait_callback(&listener, &state).await?;
    let secret = dec(&store.client_secret)?;
    let token = token_request(
        client,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", store.client_id.as_str()),
            ("client_secret", secret.as_str()),
            ("redirect_uri", REDIRECT_URI),
            ("code", code.as_str()),
        ],
    )
    .await?;
    let (uid, name) = fetch_user(client, &token.access_token).await?;
    store.access_token = enc(&token.access_token)?;
    store.refresh_token = enc(token.refresh_token.as_deref().unwrap_or(""))?;
    store.expires_at = now() + token.expires_in;
    store.uid = uid;
    store.name = name;
    save(&store)?;
    Ok(status_now())
}

#[derive(Serialize)]
pub struct ClosetItem {
    pub tid: u64,
    pub name: String,
    /// steve / alex / 其他自定义模型名
    pub kind: String,
    /// 站点现成的 3D 预览图（webp，无需鉴权）
    pub preview: String,
    /// 皮肤原图（64x64 PNG）
    pub texture: String,
}

#[derive(Serialize)]
pub struct ClosetPage {
    pub total: u64,
    pub page: u64,
    pub last_page: u64,
    pub items: Vec<ClosetItem>,
}

pub async fn closet(client: &reqwest::Client, page: u32) -> Result<ClosetPage> {
    let token = access_token(client).await?;
    let resp = client
        .get(format!("{SITE}/api/closet?page={page}&perPage=24&category=skin"))
        .header("Accept", "application/json")
        .bearer_auth(&token)
        .send()
        .await?;
    let value = api_value(resp).await?;
    let mut items = Vec::new();
    if let Some(list) = value.get("data").and_then(|v| v.as_array()) {
        for item in list {
            let hash = item.get("hash").and_then(|v| v.as_str()).unwrap_or("");
            let tid = item.get("tid").and_then(|v| v.as_u64()).unwrap_or(0);
            items.push(ClosetItem {
                tid,
                name: item
                    .get("pivot")
                    .and_then(|p| p.get("item_name"))
                    .and_then(|v| v.as_str())
                    .or_else(|| item.get("name").and_then(|v| v.as_str()))
                    .unwrap_or("")
                    .to_string(),
                kind: item
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                preview: format!("{SITE}/preview/{tid}"),
                texture: format!("{SITE}/textures/{hash}"),
            });
        }
    }
    Ok(ClosetPage {
        total: value.get("total").and_then(|v| v.as_u64()).unwrap_or(0),
        page: value
            .get("current_page")
            .and_then(|v| v.as_u64())
            .unwrap_or(page as u64),
        last_page: value
            .get("last_page")
            .and_then(|v| v.as_u64())
            .unwrap_or(1),
        items,
    })
}

#[derive(Serialize)]
pub struct PlayerView {
    pub pid: i64,
    pub name: String,
    pub tid_skin: i64,
}

pub async fn players(client: &reqwest::Client) -> Result<Vec<PlayerView>> {
    let token = access_token(client).await?;
    let resp = client
        .get(format!("{SITE}/api/players"))
        .header("Accept", "application/json")
        .bearer_auth(&token)
        .send()
        .await?;
    let value = api_value(resp).await?;
    let list = value
        .as_array()
        .cloned()
        .or_else(|| value.get("data").and_then(|v| v.as_array()).cloned())
        .unwrap_or_default();
    Ok(list
        .iter()
        .map(|item| PlayerView {
            pid: item.get("pid").and_then(|v| v.as_i64()).unwrap_or(0),
            name: item
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            tid_skin: item.get("tid_skin").and_then(|v| v.as_i64()).unwrap_or(0),
        })
        .collect())
}

/// 把衣柜里的材质应用为某个角色的皮肤。
pub async fn apply(client: &reqwest::Client, pid: i64, tid: i64) -> Result<()> {
    if pid <= 0 || tid <= 0 {
        bail!("角色或皮肤无效");
    }
    let token = access_token(client).await?;
    let resp = client
        .put(format!("{SITE}/api/players/{pid}/textures"))
        .header("Accept", "application/json")
        .bearer_auth(&token)
        .json(&serde_json::json!({ "skin": tid }))
        .send()
        .await?;
    api_value(resp).await?;
    Ok(())
}
