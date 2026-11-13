use crate::config;
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

// Prism Launcher 公开注册的 Microsoft Entra 公共客户端 ID。
// 该 ID 由 Microsoft Identity Platform 条款约束，正式分发前建议替换为自己注册的应用 ID。
const CLIENT_ID: &str = "c36a9fb6-4f2a-41ff-90bd-ae7cc92031eb";
const SCOPE: &str = "XboxLive.signin offline_access";
const DEVICE_CODE_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";

#[derive(Clone, Debug)]
pub struct Account {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub uuid: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: u64,
    pub skin_path: Option<String>,
    pub skin_model: String,
    /// 第三方认证（authlib-injector）的 Yggdrasil API 根
    pub authlib_url: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AccountView {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub uuid: String,
    pub skin_path: Option<String>,
    pub skin_model: String,
    /// 第三方认证的 API 根，前端用于展示服务名
    pub authlib_url: Option<String>,
}

impl From<&Account> for AccountView {
    fn from(a: &Account) -> Self {
        Self {
            id: a.id.clone(),
            kind: a.kind.clone(),
            name: a.name.clone(),
            uuid: a.uuid.clone(),
            skin_path: a.skin_path.clone(),
            skin_model: a.skin_model.clone(),
            authlib_url: a.authlib_url.clone(),
        }
    }
}

fn default_skin_model() -> String {
    "classic".into()
}

#[derive(Serialize, Deserialize)]
struct StoredAccount {
    id: String,
    kind: String,
    name: String,
    uuid: String,
    access_token: String,
    refresh_token: String,
    expires_at: u64,
    #[serde(default)]
    skin_path: Option<String>,
    #[serde(default = "default_skin_model")]
    skin_model: String,
    #[serde(default)]
    authlib_url: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct DeviceCode {
    #[serde(skip)]
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub message: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Deserialize)]
struct DeviceCodeResp {
    device_code: String,
    user_code: String,
    #[serde(default)]
    verification_uri: String,
    #[serde(default)]
    verification_uri_complete: String,
    #[serde(default)]
    expires_in: u64,
    #[serde(default)]
    interval: u64,
    #[serde(default)]
    message: String,
}

#[derive(Deserialize)]
struct TokenResp {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

#[derive(Deserialize)]
struct XblResp {
    #[serde(rename = "Token")]
    token: String,
    #[serde(rename = "DisplayClaims")]
    display_claims: XblClaims,
}

#[derive(Deserialize)]
struct XblClaims {
    #[serde(default)]
    xui: Vec<Xui>,
}

#[derive(Deserialize)]
struct Xui {
    #[serde(default)]
    uhs: String,
}

#[derive(Deserialize)]
struct McLoginResp {
    access_token: String,
    #[serde(default)]
    expires_in: u64,
}

#[derive(Deserialize)]
struct McProfile {
    id: String,
    name: String,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn offline_uuid(name: &str) -> String {
    let digest = md5::compute(format!("OfflinePlayer:{name}").as_bytes());
    let mut b = digest.0;
    b[6] = (b[6] & 0x0f) | 0x30;
    b[8] = (b[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    )
}

pub fn create_offline(name: &str) -> Result<Account> {
    let name = name.trim();
    if name.is_empty() {
        bail!("用户名不能为空");
    }
    if name.chars().count() > 16 {
        bail!("用户名不能超过 16 个字符");
    }
    Ok(Account {
        id: format!("offline-{}", name.to_lowercase()),
        kind: "offline".into(),
        name: name.to_string(),
        uuid: offline_uuid(name),
        access_token: "0".into(),
        refresh_token: String::new(),
        expires_at: u64::MAX,
        skin_path: None,
        skin_model: "classic".into(),
        authlib_url: None,
    })
}

pub fn load_accounts() -> Vec<Account> {
    let Ok(bytes) = std::fs::read(config::accounts_path()) else {
        return Vec::new();
    };
    let Ok(stored) = serde_json::from_slice::<Vec<StoredAccount>>(&bytes) else {
        return Vec::new();
    };
    stored
        .into_iter()
        .filter_map(|s| {
            let access = decrypt(&s.access_token).unwrap_or_else(|_| s.access_token.clone());
            let refresh = decrypt(&s.refresh_token).unwrap_or_else(|_| s.refresh_token.clone());
            Some(Account {
                id: s.id,
                kind: s.kind,
                name: s.name,
                uuid: s.uuid,
                access_token: access,
                refresh_token: refresh,
                expires_at: s.expires_at,
                skin_path: s.skin_path,
                skin_model: s.skin_model,
                authlib_url: s.authlib_url,
            })
        })
        .collect()
}

pub fn save_accounts(accounts: &[Account]) -> Result<()> {
    let stored: Vec<StoredAccount> = accounts
        .iter()
        .map(|a| StoredAccount {
            id: a.id.clone(),
            kind: a.kind.clone(),
            name: a.name.clone(),
            uuid: a.uuid.clone(),
            access_token: if a.kind == "offline" {
                a.access_token.clone()
            } else {
                encrypt(&a.access_token).unwrap_or_else(|_| a.access_token.clone())
            },
            refresh_token: if a.refresh_token.is_empty() {
                String::new()
            } else {
                encrypt(&a.refresh_token).unwrap_or_else(|_| a.refresh_token.clone())
            },
            expires_at: a.expires_at,
            skin_path: a.skin_path.clone(),
            skin_model: a.skin_model.clone(),
            authlib_url: a.authlib_url.clone(),
        })
        .collect();
    std::fs::create_dir_all(config::data_dir())?;
    std::fs::write(config::accounts_path(), serde_json::to_vec_pretty(&stored)?)?;
    Ok(())
}

fn encrypt(value: &str) -> Result<String> {
    let blob = dpapi::protect(value.as_bytes())?;
    Ok(STANDARD.encode(blob))
}

fn decrypt(value: &str) -> Result<String> {
    let blob = STANDARD.decode(value)?;
    let plain = dpapi::unprotect(&blob)?;
    Ok(String::from_utf8(plain)?)
}

pub async fn begin_device_code(client: &reqwest::Client) -> Result<DeviceCode> {
    let resp = client
        .post(DEVICE_CODE_URL)
        .form(&[("client_id", CLIENT_ID), ("scope", SCOPE)])
        .send()
        .await
        .context("无法连接微软登录服务")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!("获取设备码失败（{status}）：{text}");
    }
    let data: DeviceCodeResp = resp.json().await?;
    Ok(DeviceCode {
        device_code: data.device_code,
        user_code: data.user_code,
        verification_uri: if data.verification_uri_complete.is_empty() {
            data.verification_uri
        } else {
            data.verification_uri_complete
        },
        message: data.message,
        expires_in: data.expires_in,
        interval: data.interval.max(2),
    })
}

async fn poll_token(
    client: &reqwest::Client,
    device_code: &str,
    interval: u64,
    max_wait: u64,
) -> Result<(String, String)> {
    let deadline = now_secs() + max_wait;
    let mut wait = interval.max(2);
    loop {
        tokio::time::sleep(Duration::from_secs(wait)).await;
        if now_secs() > deadline {
            bail!("登录超时，请重新开始");
        }
        let resp = client
            .post(TOKEN_URL)
            .form(&[
                ("client_id", CLIENT_ID),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", device_code),
            ])
            .send()
            .await?;
        let status = resp.status();
        let data: TokenResp = resp.json().await.unwrap_or(TokenResp {
            access_token: None,
            refresh_token: None,
            expires_in: None,
            error: Some(format!("http-{status}")),
            error_description: None,
        });
        if let Some(token) = data.access_token {
            return Ok((token, data.refresh_token.unwrap_or_default()));
        }
        match data.error.as_deref() {
            Some("authorization_pending") => continue,
            Some("slow_down") => {
                wait += 5;
                continue;
            }
            Some("expired_token") => bail!("登录码已过期，请重新开始"),
            Some("authorization_declined") => bail!("用户拒绝了登录请求"),
            other => bail!(
                "微软登录失败：{}",
                data.error_description
                    .unwrap_or_else(|| other.unwrap_or("未知错误").to_string())
            ),
        }
    }
}

async fn exchange_ms_token(
    client: &reqwest::Client,
    ms_access_token: &str,
) -> Result<(String, String, u64, McProfile)> {
    let xbl: XblResp = client
        .post("https://user.auth.xboxlive.com/user/authenticate")
        .json(&serde_json::json!({
            "Properties": {
                "AuthMethod": "RPS",
                "SiteName": "user.auth.xboxlive.com",
                "RpsTicket": format!("d={ms_access_token}")
            },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT"
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .context("Xbox Live 认证失败")?;

    let uhs = xbl
        .display_claims
        .xui
        .first()
        .map(|x| x.uhs.clone())
        .ok_or_else(|| anyhow!("Xbox Live 未返回用户标识"))?;

    let xsts: XblResp = client
        .post("https://xsts.auth.xboxlive.com/xsts/authorize")
        .json(&serde_json::json!({
            "Properties": {
                "SandboxId": "RETAIL",
                "UserTokens": [xbl.token]
            },
            "RelyingParty": "rp://api.minecraftservices.com/",
            "TokenType": "JWT"
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .context("XSTS 授权失败（该账号可能没有 Xbox 账号或未购买游戏）")?;

    let mc: McLoginResp = client
        .post("https://api.minecraftservices.com/authentication/login_with_xbox")
        .json(&serde_json::json!({
            "identityToken": format!("XBL3.0 x={uhs};{}", xsts.token)
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .context("Minecraft 服务登录失败")?;

    let profile_resp = client
        .get("https://api.minecraftservices.com/minecraft/profile")
        .bearer_auth(&mc.access_token)
        .send()
        .await?;
    if profile_resp.status() == reqwest::StatusCode::NOT_FOUND {
        bail!("该微软账号未拥有 Minecraft: Java Edition");
    }
    let profile: McProfile = profile_resp.error_for_status()?.json().await?;

    let expires_in = if mc.expires_in == 0 { 86400 } else { mc.expires_in };
    Ok((mc.access_token, xsts.token, expires_in, profile))
}

pub async fn refresh(client: &reqwest::Client, account: &Account) -> Result<Account> {
    if account.kind != "microsoft" || account.refresh_token.is_empty() {
        return Ok(account.clone());
    }
    if account.expires_at > now_secs() + 120 {
        return Ok(account.clone());
    }
    let resp = client
        .post(TOKEN_URL)
        .form(&[
            ("client_id", CLIENT_ID),
            ("grant_type", "refresh_token"),
            ("refresh_token", account.refresh_token.as_str()),
            ("scope", SCOPE),
        ])
        .send()
        .await?;
    let data: TokenResp = resp.json().await?;
    let Some(ms_token) = data.access_token else {
        bail!(
            "登录凭据已失效，请重新登录：{}",
            data.error_description.unwrap_or_default()
        );
    };
    let new_refresh = data
        .refresh_token
        .unwrap_or_else(|| account.refresh_token.clone());
    let (mc_token, _xsts, expires_in, profile) = exchange_ms_token(client, &ms_token).await?;
    Ok(Account {
        id: account.id.clone(),
        kind: "microsoft".into(),
        name: profile.name,
        uuid: profile.id,
        access_token: mc_token,
        refresh_token: new_refresh,
        expires_at: now_secs() + expires_in,
        skin_path: account.skin_path.clone(),
        skin_model: account.skin_model.clone(),
        authlib_url: None,
    })
}

pub async fn poll_device_login(
    app: AppHandle,
    client: reqwest::Client,
    device_code: DeviceCode,
) -> Result<Account> {
    let (ms_token, refresh_token) = poll_token(
        &client,
        &device_code.device_code,
        device_code.interval,
        device_code.expires_in,
    )
    .await?;
    let (mc_token, _xsts, mc_expires, profile) = exchange_ms_token(&client, &ms_token).await?;
    let account = Account {
        id: format!("ms-{}", profile.id),
        kind: "microsoft".into(),
        name: profile.name,
        uuid: profile.id,
        access_token: mc_token,
        refresh_token,
        expires_at: now_secs() + mc_expires,
        skin_path: None,
        skin_model: "classic".into(),
        authlib_url: None,
    };
    let mut accounts = load_accounts();
    accounts.retain(|a| a.id != account.id);
    accounts.push(account.clone());
    save_accounts(&accounts)?;
    let _ = app.emit(
        "ms-login",
        serde_json::json!({ "ok": true, "account": AccountView::from(&account) }),
    );
    Ok(account)
}

#[cfg(windows)]
pub(crate) mod dpapi {
    use anyhow::{bail, Result};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB,
    };

    fn make_blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        }
    }

    pub fn protect(data: &[u8]) -> Result<Vec<u8>> {
        unsafe {
            let input = make_blob(data);
            let mut output = CRYPT_INTEGER_BLOB {
                cbData: 0,
                pbData: std::ptr::null_mut(),
            };
            let ok = CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                &mut output,
            );
            if ok == 0 {
                bail!("DPAPI 加密失败");
            }
            let result =
                std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
            LocalFree(output.pbData as *mut std::ffi::c_void);
            Ok(result)
        }
    }

    pub fn unprotect(data: &[u8]) -> Result<Vec<u8>> {
        unsafe {
            let input = make_blob(data);
            let mut output = CRYPT_INTEGER_BLOB {
                cbData: 0,
                pbData: std::ptr::null_mut(),
            };
            let ok = CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                &mut output,
            );
            if ok == 0 {
                bail!("DPAPI 解密失败");
            }
            let result =
                std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
            LocalFree(output.pbData as *mut std::ffi::c_void);
            Ok(result)
        }
    }
}

#[cfg(not(windows))]
pub(crate) mod dpapi {
    use anyhow::{bail, Result};
    pub fn protect(_data: &[u8]) -> Result<Vec<u8>> {
        bail!("仅支持 Windows")
    }
    pub fn unprotect(_data: &[u8]) -> Result<Vec<u8>> {
        bail!("仅支持 Windows")
    }
}
