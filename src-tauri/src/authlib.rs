//! authlib-injector 第三方皮肤站登录（LittleSkin 等）。
//!
//! 流程：向皮肤站的 Yggdrasil 接口换取 accessToken，启动游戏时用
//! `-javaagent:authlib-injector.jar=<API 根>` 把认证端点指到该皮肤站。

use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// LittleSkin 的 Yggdrasil API 根

static INJECTOR: OnceLock<PathBuf> = OnceLock::new();

pub fn set_injector(path: PathBuf) {
    let _ = INJECTOR.set(path);
}

pub fn injector_path() -> Option<&'static PathBuf> {
    INJECTOR.get()
}

#[derive(Debug, Clone, Serialize)]
pub struct ServerMeta {
    /// 修正后的 API 根（可能来自响应头）
    pub root: String,
    pub name: String,
    pub homepage: String,
}

#[derive(Debug, Clone)]
pub struct LoginResult {
    pub name: String,
    pub uuid: String,
    pub access_token: String,
    pub client_token: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct ProfileResp {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthResp {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    client_token: Option<String>,
    #[serde(default)]
    selected_profile: Option<ProfileResp>,
    #[serde(default)]
    available_profiles: Vec<ProfileResp>,
    // 失败响应里的错误信息：目前只用于日志/排查，保留字段便于后续展示
    #[allow(dead_code)]
    #[serde(default)]
    error: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    error_message: Option<String>,
}

fn normalize_root(root: &str) -> String {
    root.trim().trim_end_matches('/').to_string()
}

/// 每次登录用新的 clientToken，避免和同账号的其他客户端刷新冲突。
fn new_client_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id() as u128;
    let seed = nanos ^ (pid << 32);
    format!("{seed:032x}")
}

fn friendly_error(status: reqwest::StatusCode, text: &str) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
        if let Some(message) = value.get("errorMessage").and_then(|v| v.as_str()) {
            if !message.trim().is_empty() {
                return message.to_string();
            }
        }
        if let Some(code) = value.get("error").and_then(|v| v.as_str()) {
            let hint = match code {
                "ForbiddenOperationException" => "账号或密码不正确",
                "IllegalArgumentException" => "请求参数不合法",
                "UnsupportedMediaType" => "认证服务器拒绝该请求格式",
                _ => "",
            };
            if !hint.is_empty() {
                return hint.to_string();
            }
            return code.to_string();
        }
    }
    format!("认证失败（HTTP {status}）")
}

fn pick_profile(data: &AuthResp) -> Result<ProfileResp> {
    data.selected_profile
        .clone()
        .or_else(|| data.available_profiles.first().cloned())
        .ok_or_else(|| anyhow::anyhow!("该账号没有可用的角色档案"))
}

/// 读取皮肤站元数据，并按照响应头修正 API 根（LittleSkin 等会给出规范地址）。
pub async fn meta(client: &reqwest::Client, root: &str) -> Result<ServerMeta> {
    let root = normalize_root(root);
    if root.is_empty() {
        bail!("请填写认证服务器地址");
    }
    let resp = client
        .get(&root)
        .send()
        .await
        .with_context(|| format!("无法连接认证服务器：{root}"))?;
    let location = resp
        .headers()
        .get("x-authlib-injector-api-location")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim_end_matches('/').to_string());
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("认证服务器无效：{}", friendly_error(status, &text));
    }
    let json: serde_json::Value = serde_json::from_str(&text)
        .context("认证服务器返回了不是 JSON 的内容，请确认地址填的是 Yggdrasil API 根")?;
    let meta = json.get("meta");
    let name = meta
        .and_then(|value| value.get("serverName"))
        .and_then(|value| value.as_str())
        .unwrap_or("第三方认证")
        .to_string();
    let homepage = meta
        .and_then(|value| value.get("links"))
        .and_then(|value| value.get("homepage"))
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string();
    Ok(ServerMeta {
        root: location.unwrap_or(root),
        name,
        homepage,
    })
}

/// 邮箱 + 密码登录，返回角色档案与 accessToken。
pub async fn authenticate(
    client: &reqwest::Client,
    root: &str,
    username: &str,
    password: &str,
) -> Result<LoginResult> {
    let root = normalize_root(root);
    if root.is_empty() {
        bail!("请填写认证服务器地址");
    }
    if username.trim().is_empty() || password.is_empty() {
        bail!("请填写邮箱与密码");
    }
    let client_token = new_client_token();
    let resp = client
        .post(format!("{root}/authserver/authenticate"))
        .json(&serde_json::json!({
            "username": username.trim(),
            "password": password,
            "clientToken": client_token,
            "agent": { "name": "Minecraft", "version": 1 },
            "requestUser": false
        }))
        .send()
        .await
        .with_context(|| format!("无法连接认证服务器：{root}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("{}", friendly_error(status, &text));
    }
    let data: AuthResp = serde_json::from_str(&text).context("认证服务器返回了无法解析的内容")?;
    let profile = pick_profile(&data)?;
    let access_token = data
        .access_token
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("认证服务器没有返回 accessToken"))?;
    if profile.id.is_empty() {
        bail!("角色档案缺少 UUID");
    }
    Ok(LoginResult {
        name: profile.name,
        uuid: profile.id,
        access_token,
        client_token: data.client_token.unwrap_or(client_token),
    })
}

/// 用 refreshToken 续期；第三方站点的 accessToken 通常一周左右过期。
pub async fn refresh(
    client: &reqwest::Client,
    root: &str,
    access_token: &str,
    client_token: &str,
) -> Result<LoginResult> {
    let root = normalize_root(root);
    let resp = client
        .post(format!("{root}/authserver/refresh"))
        .json(&serde_json::json!({
            "accessToken": access_token,
            "clientToken": client_token,
            "requestUser": false
        }))
        .send()
        .await
        .with_context(|| format!("无法连接认证服务器：{root}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("{}", friendly_error(status, &text));
    }
    let data: AuthResp = serde_json::from_str(&text).context("认证服务器返回了无法解析的内容")?;
    let profile = pick_profile(&data)?;
    let access_token = data
        .access_token
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("认证服务器没有返回 accessToken"))?;
    Ok(LoginResult {
        name: profile.name,
        uuid: profile.id,
        access_token,
        client_token: data.client_token.unwrap_or_else(|| client_token.to_string()),
    })
}

fn injector_file() -> PathBuf {
    crate::config::data_dir().join("authlib-injector.jar")
}

/// 确保本地有 authlib-injector，返回 jar 路径（只下载一次）。
pub async fn ensure_injector(client: &reqwest::Client) -> Result<PathBuf> {
    let path = injector_file();
    if std::fs::metadata(&path)
        .map(|meta| meta.len() > 100_000)
        .unwrap_or(false)
    {
        return Ok(path);
    }
    std::fs::create_dir_all(crate::config::data_dir())?;

    let mut version = "1.2.8".to_string();
    // latest.json 里 artifact 路径用的是 build_number，直接优先用它的 download_url
    let mut candidates: Vec<String> = Vec::new();
    if let Ok(bytes) = crate::download::get_bytes(
        client,
        "https://authlib-injector.yushi.moe/artifact/latest.json",
        false,
    )
    .await
    {
        if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            if let Some(value) = json.get("version").and_then(|value| value.as_str()) {
                if !value.trim().is_empty() {
                    version = value.to_string();
                }
            }
            if let Some(url) = json.get("download_url").and_then(|value| value.as_str()) {
                if url.starts_with("https://") {
                    candidates.push(url.to_string());
                }
            }
            if let Some(build) = json.get("build_number").and_then(|value| value.as_i64()) {
                candidates.push(format!(
                    "https://authlib-injector.yushi.moe/artifact/{build}/authlib-injector-{version}.jar"
                ));
            }
        }
    }
    candidates.push(format!(
        "https://github.com/yushijinhun/authlib-injector/releases/download/v{version}/authlib-injector-{version}.jar"
    ));
    candidates.push(
        "https://authlib-injector.yushi.moe/artifact/latest/authlib-injector.jar".to_string(),
    );
    let mut last = String::new();
    for url in candidates {
        match crate::download::get_bytes(client, &url, false).await {
            Ok(bytes) if bytes.len() > 100_000 => {
                std::fs::write(&path, &bytes)?;
                return Ok(path);
            }
            Ok(_) => last = "文件不完整".into(),
            Err(e) => last = e.to_string(),
        }
    }
    bail!("下载 authlib-injector 失败：{last}")
}

/// 启动前预热：第三方账号才需要。
pub async fn prepare(client: &reqwest::Client, account: &crate::auth::Account) {
    if account.kind != "authlib" {
        return;
    }
    if injector_path().is_some() {
        return;
    }
    if let Ok(path) = ensure_injector(client).await {
        set_injector(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Yggdrasil 返回的是 camelCase，解析失败会误报“没有可用的角色档案”
    #[test]
    fn parses_camel_case_auth_response() {
        let text = r#"{
            "accessToken": "token",
            "clientToken": "client",
            "selectedProfile": { "id": "069a79f444e94726a5befca90e38aaf5", "name": "Steve" },
            "availableProfiles": [{ "id": "069a79f444e94726a5befca90e38aaf5", "name": "Steve" }]
        }"#;
        let data: AuthResp = serde_json::from_str(text).unwrap();
        assert_eq!(data.access_token.as_deref(), Some("token"));
        assert_eq!(data.client_token.as_deref(), Some("client"));
        let profile = pick_profile(&data).unwrap();
        assert_eq!(profile.name, "Steve");
        assert_eq!(profile.id, "069a79f444e94726a5befca90e38aaf5");
    }

    /// 没有 selectedProfile 时退回第一个可用角色
    #[test]
    fn falls_back_to_first_available_profile() {
        let data: AuthResp = serde_json::from_str(
            r#"{"availableProfiles":[{"id":"abc","name":"Alex"}]}"#,
        )
        .unwrap();
        assert_eq!(pick_profile(&data).unwrap().name, "Alex");
    }
}
