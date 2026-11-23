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

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct Profile {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct LoginResult {
    pub name: String,
    pub uuid: String,
    pub access_token: String,
    pub client_token: String,
    /// 该账号在皮肤站上的全部角色（availableProfiles）
    pub profiles: Vec<Profile>,
    /// token 当前绑定的角色；None 表示需要启动器选择角色
    pub selected: Option<Profile>,
}

/// 等待用户选择角色的登录会话：密码不落盘，只保留这次换到的 token。
#[derive(Debug, Clone)]
pub struct PendingLogin {
    pub root: String,
    pub login: LoginResult,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthResp {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    client_token: Option<String>,
    #[serde(default)]
    selected_profile: Option<Profile>,
    #[serde(default)]
    available_profiles: Vec<Profile>,
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

/// 合并 availableProfiles 与 selectedProfile，按 UUID 去重并过滤空档案。
fn collect_profiles(data: &AuthResp) -> Vec<Profile> {
    let mut out: Vec<Profile> = Vec::new();
    let mut push = |profile: &Profile| {
        if profile.id.is_empty() || out.iter().any(|item| item.id == profile.id) {
            return;
        }
        out.push(profile.clone());
    };
    for profile in &data.available_profiles {
        push(profile);
    }
    if let Some(profile) = &data.selected_profile {
        push(profile);
    }
    out
}

fn login_result(data: AuthResp, fallback_client_token: String) -> Result<LoginResult> {
    let profiles = collect_profiles(&data);
    let selected = data
        .selected_profile
        .clone()
        .filter(|profile| !profile.id.is_empty());
    // token 没绑定角色时先拿第一个当展示用，真正进游戏前仍需用户选择
    let profile = selected
        .clone()
        .or_else(|| profiles.first().cloned())
        .ok_or_else(|| anyhow::anyhow!("该账号没有可用的角色档案"))?;
    let access_token = data
        .access_token
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("认证服务器没有返回 accessToken"))?;
    Ok(LoginResult {
        name: profile.name,
        uuid: profile.id,
        access_token,
        client_token: data.client_token.unwrap_or(fallback_client_token),
        profiles,
        selected,
    })
}

/// 请求体里统一带上 agent，第三方实现对该字段的要求并不一致。
fn login_body(username: &str, password: &str, client_token: &str) -> serde_json::Value {
    serde_json::json!({
        "username": username,
        "password": password,
        "clientToken": client_token,
        "agent": { "name": "Minecraft", "version": 1 },
        "requestUser": false
    })
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
        .json(&login_body(username.trim(), password, &client_token))
        .send()
        .await
        .with_context(|| format!("无法连接认证服务器：{root}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("{}", friendly_error(status, &text));
    }
    let data: AuthResp = serde_json::from_str(&text).context("认证服务器返回了无法解析的内容")?;
    if collect_profiles(&data).is_empty() {
        bail!("该账号没有可用的角色档案");
    }
    login_result(data, client_token)
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
    login_result(data, client_token.to_string())
}

/// 在多个角色之间做选择：按 Yggdrasil 规范，refresh 请求带 `selectedProfile`
/// 会把新 token 绑定到该角色（原 token 必须尚未绑定）。
pub async fn select_profile(
    client: &reqwest::Client,
    root: &str,
    access_token: &str,
    client_token: &str,
    profile: &Profile,
) -> Result<LoginResult> {
    let root = normalize_root(root);
    if profile.id.is_empty() {
        bail!("角色信息不完整，请重新登录");
    }
    let resp = client
        .post(format!("{root}/authserver/refresh"))
        .json(&serde_json::json!({
            "accessToken": access_token,
            "clientToken": client_token,
            "requestUser": false,
            "selectedProfile": { "id": profile.id, "name": profile.name }
        }))
        .send()
        .await
        .with_context(|| format!("无法连接认证服务器：{root}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        let message = friendly_error(status, &text);
        // 已经绑定过角色的 token 不能再改绑，提示用户重新登录更直接
        bail!(
            "{message}。如果仍无法切换，请在皮肤站里更换默认角色，或移除该账号后重新登录并选择角色"
        );
    }
    let mut data: AuthResp =
        serde_json::from_str(&text).context("认证服务器返回了无法解析的内容")?;
    if data.selected_profile.is_none() {
        data.selected_profile = Some(profile.clone());
    }
    login_result(data, client_token.to_string())
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
        let login = login_result(data, "fallback".into()).unwrap();
        assert_eq!(login.name, "Steve");
        assert_eq!(login.uuid, "069a79f444e94726a5befca90e38aaf5");
        assert_eq!(login.client_token, "client");
        assert_eq!(login.profiles.len(), 1);
        assert!(login.selected.is_some());
    }

    /// 没有 selectedProfile 时退回第一个可用角色，但标记为「待选择」
    #[test]
    fn falls_back_to_first_available_profile() {
        let data: AuthResp = serde_json::from_str(
            r#"{"accessToken":"t","availableProfiles":[{"id":"abc","name":"Alex"}]}"#,
        )
        .unwrap();
        let login = login_result(data, "c".into()).unwrap();
        assert_eq!(login.name, "Alex");
        assert_eq!(login.uuid, "abc");
        assert!(login.selected.is_none());
    }

    /// 多角色账号要保留全部档案，供登录后选择
    #[test]
    fn keeps_all_available_profiles() {
        let data: AuthResp = serde_json::from_str(
            r#"{"accessToken":"t","availableProfiles":[
                {"id":"a1","name":"Alice"},
                {"id":"b2","name":"Bob"},
                {"id":"a1","name":"Alice"}
            ]}"#,
        )
        .unwrap();
        let login = login_result(data, "c".into()).unwrap();
        assert_eq!(login.profiles.len(), 2);
        assert_eq!(login.profiles[1].name, "Bob");
        assert!(login.selected.is_none());
    }
}
