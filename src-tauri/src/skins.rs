use crate::auth::{self, Account};
use crate::config;
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const SKIN_SIZE: u32 = 64;
const MAX_SKIN_BYTES: usize = 2 * 1024 * 1024;

pub fn skins_dir() -> PathBuf {
    config::data_dir().join("skins")
}

fn safe_id(account_id: &str) -> String {
    account_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub fn local_skin_path(account_id: &str) -> PathBuf {
    skins_dir().join(format!("{}.png", safe_id(account_id)))
}

pub fn to_data_url(bytes: &[u8], mime: &str) -> String {
    format!("data:{mime};base64,{}", STANDARD.encode(bytes))
}

/// 内置的 Steve 皮肤，离线账号与未登录时使用。
pub fn default_skin() -> String {
    let entry = DEFAULT_SKINS
        .iter()
        .find(|skin| skin.id == "steve")
        .unwrap_or(&DEFAULT_SKINS[0]);
    let bytes = normalize_png(entry.bytes.to_vec()).unwrap_or_else(|_| entry.bytes.to_vec());
    to_data_url(&bytes, "image/png")
}

pub struct DefaultSkinEntry {
    pub id: &'static str,
    pub name: &'static str,
    pub model: &'static str,
    pub bytes: &'static [u8],
}

const DEFAULT_SKINS: &[DefaultSkinEntry] = &[
    DefaultSkinEntry {
        id: "steve",
        name: "Steve",
        model: "classic",
        bytes: include_bytes!("../assets/skins/steve.png"),
    },
    DefaultSkinEntry {
        id: "alex",
        name: "Alex",
        model: "slim",
        bytes: include_bytes!("../assets/skins/alex.png"),
    },
    DefaultSkinEntry {
        id: "ari",
        name: "Ari",
        model: "slim",
        bytes: include_bytes!("../assets/skins/ari.png"),
    },
    DefaultSkinEntry {
        id: "kai",
        name: "Kai",
        model: "classic",
        bytes: include_bytes!("../assets/skins/kai.png"),
    },
    DefaultSkinEntry {
        id: "noor",
        name: "Noor",
        model: "classic",
        bytes: include_bytes!("../assets/skins/noor.png"),
    },
    DefaultSkinEntry {
        id: "sunny",
        name: "Sunny",
        model: "classic",
        bytes: include_bytes!("../assets/skins/sunny.png"),
    },
    DefaultSkinEntry {
        id: "zuri",
        name: "Zuri",
        model: "classic",
        bytes: include_bytes!("../assets/skins/zuri.png"),
    },
    DefaultSkinEntry {
        id: "makena",
        name: "Makena",
        model: "classic",
        bytes: include_bytes!("../assets/skins/makena.png"),
    },
    DefaultSkinEntry {
        id: "efe",
        name: "Efe",
        model: "classic",
        bytes: include_bytes!("../assets/skins/efe.png"),
    },
];

fn default_index(seed: &str) -> usize {
    let mut hash: u32 = 2166136261;
    for byte in seed.bytes() {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(16777619);
    }
    hash as usize % DEFAULT_SKINS.len()
}

/// 每个账号固定挑一套默认皮肤，避免所有离线账号长一样。
pub fn default_skin_for(seed: &str) -> (String, String) {
    let entry = &DEFAULT_SKINS[default_index(seed)];
    let bytes = normalize_png(entry.bytes.to_vec()).unwrap_or_else(|_| entry.bytes.to_vec());
    (to_data_url(&bytes, "image/png"), entry.model.to_string())
}

/// 内置默认皮肤的字节，用于识别旧版本给第三方账号预装的占位皮肤。
fn builtin_skin_bytes(seed: &str) -> Option<Vec<u8>> {
    let entry = &DEFAULT_SKINS[default_index(seed)];
    normalize_png(entry.bytes.to_vec()).ok()
}

pub fn install_default(account_id: &str, seed: &str) -> Result<(PathBuf, String)> {
    let entry = &DEFAULT_SKINS[default_index(seed)];
    let bytes = normalize_png(entry.bytes.to_vec()).context("默认皮肤损坏")?;
    std::fs::create_dir_all(skins_dir())?;
    let path = local_skin_path(account_id);
    std::fs::write(&path, bytes)?;
    Ok((path, entry.model.to_string()))
}

#[derive(serde::Serialize)]
pub struct DefaultSkinView {
    pub id: String,
    pub name: String,
    pub model: String,
    pub data_url: String,
}

pub fn default_skins() -> Vec<DefaultSkinView> {
    DEFAULT_SKINS
        .iter()
        .map(|entry| {
            let bytes = normalize_png(entry.bytes.to_vec())
                .unwrap_or_else(|_| entry.bytes.to_vec());
            DefaultSkinView {
                id: entry.id.to_string(),
                name: entry.name.to_string(),
                model: entry.model.to_string(),
                data_url: to_data_url(&bytes, "image/png"),
            }
        })
        .collect()
}

fn decode_base64(raw: &str) -> Result<Vec<u8>> {
    let value = raw
        .split_once(',')
        .map(|(_, rest)| rest)
        .unwrap_or(raw)
        .trim();
    if value.len() > MAX_SKIN_BYTES * 2 {
        bail!("皮肤文件过大（上限 2 MB）");
    }
    STANDARD.decode(value).context("皮肤 base64 解码失败")
}

/// 校验 PNG 并统一为 64×64。64×32 旧皮肤会补成现代布局。
pub fn normalize_png(bytes: Vec<u8>) -> Result<Vec<u8>> {
    if bytes.len() > MAX_SKIN_BYTES {
        bail!("皮肤文件过大（上限 2 MB）");
    }
    if bytes.len() < 33 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        bail!("不是有效的 PNG 图片");
    }
    let decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
    let mut reader = decoder.read_info().context("解码 PNG 失败")?;
    let info = reader.info();
    let (width, height) = (info.width, info.height);
    if width != SKIN_SIZE || (height != SKIN_SIZE && height != 32) {
        bail!("皮肤尺寸必须是 64×64 或 64×32，当前为 {width}×{height}");
    }
    if height == SKIN_SIZE {
        return Ok(bytes);
    }

    let mut buffer = vec![0; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut buffer)?;
    buffer.truncate(frame.buffer_size());
    let source = buffer;

    let mut out = vec![0u8; (SKIN_SIZE * SKIN_SIZE * 4) as usize];
    let copy_rect = |out: &mut Vec<u8>, sx: u32, sy: u32, w: u32, h: u32, dx: u32, dy: u32| {
        for y in 0..h {
            for x in 0..w {
                let src = ((sy + y) * width + sx + x) as usize * 4;
                let dst = ((dy + y) * SKIN_SIZE + dx + x) as usize * 4;
                if src + 3 < source.len() && dst + 3 < out.len() {
                    out[dst..dst + 4].copy_from_slice(&source[src..src + 4]);
                }
            }
        }
    };
    // 64×32 的头部/帽子/身体/右臂/右腿平移到现代布局
    copy_rect(&mut out, 0, 0, 32, 16, 0, 0);
    // 帽子图层（头发、头盔等）在旧版布局里同样位于 (32,0)，之前漏掉会让皮肤看起来像换了个人
    copy_rect(&mut out, 32, 0, 32, 16, 32, 0);
    copy_rect(&mut out, 16, 16, 24, 16, 16, 16);
    copy_rect(&mut out, 40, 16, 16, 16, 40, 16);
    copy_rect(&mut out, 0, 16, 16, 16, 0, 16);
    // 左腿/左臂由右腿/右臂水平镜像生成
    for y in 0..16u32 {
        for x in 0..16u32 {
            let src = ((16 + y) * width + x) as usize * 4;
            let dst = ((48 + y) * SKIN_SIZE + (16 + (15 - x))) as usize * 4;
            if src + 3 < source.len() && dst + 3 < out.len() {
                out[dst..dst + 4].copy_from_slice(&source[src..src + 4]);
            }
            let src_arm = ((16 + y) * width + 40 + x) as usize * 4;
            let dst_arm = ((48 + y) * SKIN_SIZE + (32 + (15 - x))) as usize * 4;
            if src_arm + 3 < source.len() && dst_arm + 3 < out.len() {
                out[dst_arm..dst_arm + 4].copy_from_slice(&source[src_arm..src_arm + 4]);
            }
        }
    }

    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, SKIN_SIZE, SKIN_SIZE);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&out)?;
    }
    Ok(encoded)
}

pub fn decode_and_normalize(raw: &str) -> Result<Vec<u8>> {
    normalize_png(decode_base64(raw)?)
}

pub fn save_local(account_id: &str, raw_base64: &str) -> Result<PathBuf> {
    let bytes = decode_and_normalize(raw_base64)?;
    std::fs::create_dir_all(skins_dir())?;
    let path = local_skin_path(account_id);
    std::fs::write(&path, bytes)?;
    Ok(path)
}

pub fn read_local_bytes(path: &str) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

pub fn current_bytes(account: &Account) -> Option<Vec<u8>> {
    account
        .skin_path
        .as_ref()
        .and_then(|path| read_local_bytes(path))
}

#[derive(Deserialize)]
struct ProfileProperty {
    #[serde(default)]
    name: String,
    #[serde(default)]
    value: String,
}

#[derive(Deserialize)]
struct ProfileResponse {
    #[serde(default)]
    properties: Vec<ProfileProperty>,
}

#[derive(Deserialize)]
struct TexturePayload {
    textures: TextureMap,
}

#[derive(Deserialize)]
struct TextureMap {
    #[serde(rename = "SKIN")]
    skin: Option<SkinTexture>,
    #[serde(rename = "CAPE")]
    cape: Option<SkinTexture>,
}

#[derive(Deserialize)]
struct SkinTexture {
    url: String,
    #[serde(default)]
    metadata: Option<SkinMetadata>,
}

#[derive(Deserialize)]
struct SkinMetadata {
    #[serde(default)]
    model: String,
}

/// 读取 Yggdrasil/sessionserver 档案并解出 textures 属性。
async fn profile_textures(client: &reqwest::Client, url: &str) -> Result<TextureMap> {
    let profile: ProfileResponse = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .context("读取皮肤档案失败")?;

    let property = profile
        .properties
        .iter()
        .find(|p| p.name == "textures")
        .ok_or_else(|| anyhow!("档案里没有皮肤信息"))?;
    let decoded = STANDARD
        .decode(property.value.as_bytes())
        .context("皮肤属性解码失败")?;
    let payload: TexturePayload =
        serde_json::from_slice(&decoded).context("皮肤属性解析失败")?;
    Ok(payload.textures)
}

/// 读取档案里的皮肤并下载（Mojang 与第三方皮肤站格式一致）。
async fn skin_from_profile(client: &reqwest::Client, url: &str) -> Result<(Vec<u8>, String)> {
    let textures = profile_textures(client, url).await?;
    let skin = textures
        .skin
        .ok_or_else(|| anyhow!("该账号没有设置皮肤"))?;
    let model = skin
        .metadata
        .as_ref()
        .map(|m| m.model.clone())
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| "classic".into());
    let bytes = client
        .get(&skin.url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?
        .to_vec();
    Ok((bytes, model))
}

pub async fn official_skin(client: &reqwest::Client, uuid: &str) -> Result<(Vec<u8>, String)> {
    skin_from_profile(
        client,
        &format!("https://sessionserver.mojang.com/session/minecraft/profile/{uuid}"),
    )
    .await
}

/// 第三方皮肤站（LittleSkin 等）的 Yggdrasil 档案接口。
pub async fn authlib_skin(
    client: &reqwest::Client,
    root: &str,
    uuid: &str,
) -> Result<(Vec<u8>, String)> {
    let root = root.trim().trim_end_matches('/');
    if root.is_empty() || uuid.is_empty() {
        bail!("皮肤站地址或角色 UUID 为空");
    }
    skin_from_profile(
        client,
        &format!("{root}/sessionserver/session/minecraft/profile/{uuid}"),
    )
    .await
}

pub async fn fallback_skin(
    client: &reqwest::Client,
    uuid: &str,
) -> Result<Vec<u8>> {
    let bytes = client
        .get(format!("https://mc-heads.net/skin/{uuid}"))
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?
        .to_vec();
    Ok(bytes)
}

/// 账号页 3D 预览所需的皮肤/披风数据。
pub struct LoadedSkin {
    pub data_url: String,
    pub model: String,
    pub source: String,
}

pub async fn load_skin(client: &reqwest::Client, account: &Account) -> LoadedSkin {
    if let Some(bytes) = current_bytes(account) {
        let model = if account.skin_model.is_empty() {
            "classic".to_string()
        } else {
            account.skin_model.clone()
        };
        // 旧版本给第三方账号写过内置默认皮肤作占位，不能让它在皮肤站皮肤之前生效
        let placeholder = account.kind == "authlib"
            && builtin_skin_bytes(&account.id)
                .map(|builtin| builtin == bytes)
                .unwrap_or(false);
        if !placeholder {
            return LoadedSkin {
                data_url: to_data_url(&bytes, "image/png"),
                model,
                source: "local".into(),
            };
        }
    }
    if account.kind == "authlib" {
        if let Some(root) = account
            .authlib_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if let Ok((bytes, model)) = authlib_skin(client, root, &account.uuid).await {
                return LoadedSkin {
                    data_url: to_data_url(&bytes, "image/png"),
                    model,
                    source: "authlib".into(),
                };
            }
        }
    }
    if account.kind == "microsoft" {
        if let Ok((bytes, model)) = official_skin(client, &account.uuid).await {
            return LoadedSkin {
                data_url: to_data_url(&bytes, "image/png"),
                model,
                source: "official".into(),
            };
        }
        if let Ok(bytes) = fallback_skin(client, &account.uuid).await {
            return LoadedSkin {
                data_url: to_data_url(&bytes, "image/png"),
                model: "classic".into(),
                source: "official".into(),
            };
        }
    }
    let (data_url, model) = default_skin_for(&account.id);
    LoadedSkin {
        data_url,
        model,
        source: "default".into(),
    }
}

pub async fn upload_skin(
    client: &reqwest::Client,
    account: &Account,
    raw_base64: &str,
    model: &str,
) -> Result<()> {
    if account.kind != "microsoft" {
        bail!("只有微软账号可以上传官方皮肤");
    }
    let bytes = decode_and_normalize(raw_base64)?;
    let refreshed = auth::refresh(client, account)
        .await
        .context("刷新微软登录凭据失败")?;
    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name("skin.png")
        .mime_str("image/png")?;
    let form = reqwest::multipart::Form::new()
        .text("variant", model.to_string())
        .part("file", part);
    let response = client
        .put("https://api.minecraftservices.com/minecraft/profile/skins")
        .bearer_auth(&refreshed.access_token)
        .multipart(form)
        .send()
        .await?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let text = response.text().await.unwrap_or_default();
    let message = match status.as_u16() {
        401 => "登录凭据已失效，请重新登录微软账号".to_string(),
        403 => "Mojang 拒绝上传（账号可能未拥有 Java 版或令牌权限不足）".to_string(),
        429 => "上传过于频繁，请稍后再试".to_string(),
        _ => format!("上传失败（{status}）：{text}"),
    };
    bail!(message)
}

pub fn export_bytes(account: &Account, target: &str) -> Result<()> {
    let bytes = current_bytes(account).ok_or_else(|| anyhow!("该账号还没有本地皮肤"))?;
    if let Some(parent) = std::path::Path::new(target).parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(target, bytes)?;
    Ok(())
}

// ---------- 披风：本地库 / 导入导出 / 第三方上传 ----------

const CAPE_WIDTH: u32 = 64;
const CAPE_HEIGHT: u32 = 32;

fn capes_dir() -> PathBuf {
    skins_dir().join("capes")
}

/// 披风只接受 64×32 的 PNG；皮肤那套 64×32→64×64 的旧版补全不适用于披风
fn normalize_cape_png(bytes: Vec<u8>) -> Result<Vec<u8>> {
    if bytes.len() > MAX_SKIN_BYTES {
        bail!("披风文件过大（上限 2 MB）");
    }
    if bytes.len() < 33 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        bail!("不是有效的 PNG 图片");
    }
    let decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
    let reader = decoder.read_info().context("解码 PNG 失败")?;
    let info = reader.info();
    if info.width != CAPE_WIDTH || info.height != CAPE_HEIGHT {
        bail!(
            "披风尺寸必须是 {CAPE_WIDTH}×{CAPE_HEIGHT}，当前为 {}×{}",
            info.width,
            info.height
        );
    }
    Ok(bytes)
}

fn decode_cape_base64(raw: &str) -> Result<Vec<u8>> {
    normalize_cape_png(decode_base64(raw)?)
}

fn cape_path(name: &str) -> PathBuf {
    capes_dir().join(format!("{}.png", safe_id(name)))
}

#[derive(Debug, Clone, Serialize)]
pub struct LocalCapeView {
    pub name: String,
    pub saved_at: i64,
    pub data_url: String,
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn save_local_cape(name: &str, raw_base64: &str) -> Result<LocalCapeView> {
    let name = name.trim();
    if name.is_empty() {
        bail!("披风名称不能为空");
    }
    let name: String = name.chars().take(32).collect();
    let bytes = decode_cape_base64(raw_base64)?;
    std::fs::create_dir_all(capes_dir())?;
    std::fs::write(cape_path(&name), &bytes).context("保存披风失败")?;
    Ok(LocalCapeView {
        name,
        saved_at: now_secs(),
        data_url: to_data_url(&bytes, "image/png"),
    })
}

pub fn list_local_capes() -> Vec<LocalCapeView> {
    let Ok(entries) = std::fs::read_dir(capes_dir()) else {
        return Vec::new();
    };
    let mut list: Vec<LocalCapeView> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().map(|ext| ext != "png").unwrap_or(true) {
                return None;
            }
            let bytes = std::fs::read(&path).ok()?;
            if normalize_cape_png(bytes.clone()).is_err() {
                return None;
            }
            let saved_at = entry
                .metadata()
                .ok()
                .and_then(|meta| meta.modified().ok())
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            Some(LocalCapeView {
                name: path.file_stem()?.to_string_lossy().to_string(),
                saved_at,
                data_url: to_data_url(&bytes, "image/png"),
            })
        })
        .collect();
    list.sort_by(|a, b| b.saved_at.cmp(&a.saved_at));
    list
}

pub fn remove_local_cape(name: &str) -> Result<()> {
    let path = cape_path(name);
    if !path.is_file() {
        bail!("披风不存在：{name}");
    }
    std::fs::remove_file(path).context("删除披风失败")
}

/// 导出为 PNG 文件
pub fn export_cape_file(target: &str, raw_base64: &str) -> Result<()> {
    let bytes = decode_cape_base64(raw_base64)?;
    if let Some(parent) = std::path::Path::new(target).parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(target, bytes).context("写入披风文件失败")
}

/// 从本地 PNG 导入，返回 data URL 供画布直接加载
pub fn import_cape_file(path: &str) -> Result<String> {
    let bytes = std::fs::read(path).context("读取披风文件失败")?;
    let bytes = normalize_cape_png(bytes)?;
    Ok(to_data_url(&bytes, "image/png"))
}

/// 材质上传接口挂在站点根上（`{站点}/api/user/profile/{uuid}/{type}`），
/// 而 authlib_url 通常带着 `/api/yggdrasil` 前缀，需要先退回站点根再拼路径。
fn site_root(authlib_url: &str) -> String {
    let root = authlib_url.trim().trim_end_matches('/');
    for suffix in ["/api/yggdrasil", "/api/yggdrasil/v2", "/api"] {
        if let Some(stripped) = root.strip_suffix(suffix) {
            return stripped.trim_end_matches('/').to_string();
        }
    }
    root.to_string()
}

/// 上传到 authlib 第三方皮肤站（LittleSkin 等）。
/// 微软账号没有自定义披风上传接口，只能选择账号已拥有的披风。
pub async fn upload_cape(
    client: &reqwest::Client,
    account: &Account,
    raw_base64: &str,
) -> Result<()> {
    let bytes = decode_cape_base64(raw_base64)?;
    let Some(root) = account
        .authlib_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        bail!("微软账号不支持上传自定义披风，请在账号页选择已拥有的披风");
    };
    if account.access_token.is_empty() {
        bail!("登录凭据缺失，请先重新登录该皮肤站账号");
    }
    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name("cape.png")
        .mime_str("image/png")?;
    let form = reqwest::multipart::Form::new().part("file", part);
    let url = format!(
        "{}/api/user/profile/{}/cape",
        site_root(root),
        account.uuid
    );
    let response = client
        .put(url)
        .bearer_auth(&account.access_token)
        .multipart(form)
        .send()
        .await
        .context("连接皮肤站失败")?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let text = response.text().await.unwrap_or_default();
    let message = match status.as_u16() {
        401 | 403 => "皮肤站拒绝上传：登录已过期或令牌权限不足，请重新登录该账号".to_string(),
        404 => format!(
            "该皮肤站没有开放材质上传接口，请到 {}/user/player 网页上传披风",
            site_root(root)
        ),
        413 => "披风文件超出皮肤站限制".to_string(),
        _ => format!("上传失败（{status}）：{text}"),
    };
    bail!(message)
}

// ---------- 皮肤库 ----------

fn library_dir() -> PathBuf {
    skins_dir().join("library")
}

fn library_index_path() -> PathBuf {
    library_dir().join("library.json")
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct LibraryEntry {
    pub id: String,
    pub name: String,
    pub model: String,
    pub source: String,
    pub created_at: i64,
}

#[derive(serde::Serialize)]
pub struct LibraryItemView {
    pub id: String,
    pub name: String,
    pub model: String,
    pub source: String,
    pub created_at: i64,
    pub data_url: String,
}

pub fn load_library() -> Vec<LibraryEntry> {
    let raw = match std::fs::read(library_index_path()) {
        Ok(raw) => raw,
        Err(_) => return Vec::new(),
    };
    serde_json::from_slice(&raw).unwrap_or_default()
}

fn save_library(entries: &[LibraryEntry]) -> Result<()> {
    std::fs::create_dir_all(library_dir())?;
    let raw = serde_json::to_vec_pretty(entries)?;
    std::fs::write(library_index_path(), raw)?;
    Ok(())
}

fn library_png_path(id: &str) -> PathBuf {
    library_dir().join(format!("{}.png", safe_id(id)))
}

fn new_library_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}", nanos)
}

pub fn library_add(
    name: &str,
    model: &str,
    bytes: Vec<u8>,
    source: &str,
) -> Result<LibraryEntry> {
    let name = name.trim();
    if name.is_empty() {
        bail!("名称不能为空");
    }
    let bytes = normalize_png(bytes)?;
    let entry = LibraryEntry {
        id: new_library_id(),
        name: name.chars().take(32).collect(),
        model: if model == "slim" { "slim".into() } else { "classic".into() },
        source: source.to_string(),
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
    };
    std::fs::create_dir_all(library_dir())?;
    std::fs::write(library_png_path(&entry.id), bytes)?;
    let mut entries = load_library();
    entries.retain(|item| item.name != entry.name);
    entries.push(entry.clone());
    entries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    save_library(&entries)?;
    Ok(entry)
}

pub fn library_rename(id: &str, name: &str) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        bail!("名称不能为空");
    }
    let mut entries = load_library();
    let entry = entries
        .iter_mut()
        .find(|item| item.id == id)
        .ok_or_else(|| anyhow!("皮肤不存在"))?;
    entry.name = name.chars().take(32).collect();
    save_library(&entries)
}

pub fn library_remove(id: &str) -> Result<()> {
    let mut entries = load_library();
    let before = entries.len();
    entries.retain(|item| item.id != id);
    if entries.len() == before {
        bail!("皮肤不存在");
    }
    save_library(&entries)?;
    let _ = std::fs::remove_file(library_png_path(id));
    Ok(())
}

pub fn library_bytes(id: &str) -> Result<Vec<u8>> {
    std::fs::read(library_png_path(id)).context("读取皮肤文件失败")
}

pub fn library_views() -> Vec<LibraryItemView> {
    load_library()
        .into_iter()
        .filter_map(|entry| {
            let bytes = library_bytes(&entry.id).ok()?;
            Some(LibraryItemView {
                id: entry.id,
                name: entry.name,
                model: entry.model,
                source: entry.source,
                created_at: entry.created_at,
                data_url: to_data_url(&bytes, "image/png"),
            })
        })
        .collect()
}

// ---------- 按玩家名抓取 ----------

#[derive(Deserialize)]
struct PlayerLookup {
    id: String,
    name: String,
}

/// 通过 Mojang 官方接口拉取指定玩家的当前皮肤，返回（玩家名，皮肤字节，模型）。
pub async fn player_skin(client: &reqwest::Client, name: &str) -> Result<(String, Vec<u8>, String)> {
    let name = name.trim();
    if name.len() < 3 || name.len() > 16 || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        bail!("玩家名格式不正确（3-16 位字母、数字或下划线）");
    }
    let response = client
        .get(format!(
            "https://api.mojang.com/users/profiles/minecraft/{name}"
        ))
        .send()
        .await?;
    if response.status().as_u16() == 404 || response.status().as_u16() == 204 {
        bail!("玩家不存在（仅限正版玩家名）");
    }
    let lookup: PlayerLookup = response
        .error_for_status()?
        .json()
        .await
        .context("解析玩家信息失败")?;
    let (bytes, model) = official_skin(client, &lookup.id)
        .await
        .context("该玩家没有设置皮肤")?;
    let bytes = normalize_png(bytes)?;
    Ok((lookup.name, bytes, model))
}

// ---------- 披风 ----------

#[derive(serde::Serialize)]
pub struct CapeView {
    pub id: String,
    pub alias: String,
    pub url: String,
    pub active: bool,
}

#[derive(Deserialize)]
struct CapeInfo {
    #[serde(default)]
    id: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    alias: String,
}

#[derive(Deserialize)]
struct ProfileCapes {
    #[serde(default)]
    capes: Vec<CapeInfo>,
}

pub async fn list_capes(client: &reqwest::Client, account: &Account) -> Result<Vec<CapeView>> {
    // 第三方账号：披风由皮肤站的角色档案下发，这里只读展示当前披风
    if account.kind == "authlib" {
        let root = account
            .authlib_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("第三方账号缺少皮肤站地址"))?;
        let textures = profile_textures(
            client,
            &format!(
                "{}/sessionserver/session/minecraft/profile/{}",
                root.trim_end_matches('/'),
                account.uuid
            ),
        )
        .await?;
        return Ok(textures
            .cape
            .map(|cape| {
                vec![CapeView {
                    id: "authlib".into(),
                    alias: "当前披风".into(),
                    url: cape.url,
                    active: true,
                }]
            })
            .unwrap_or_default());
    }
    if account.kind != "microsoft" {
        bail!("离线账号没有披风");
    }
    let refreshed = auth::refresh(client, account)
        .await
        .context("刷新微软登录凭据失败")?;
    let profile: ProfileCapes = client
        .get("https://api.minecraftservices.com/minecraft/profile")
        .bearer_auth(&refreshed.access_token)
        .send()
        .await?
        .error_for_status()
        .context("读取披风列表失败")?
        .json()
        .await
        .context("解析披风列表失败")?;
    Ok(profile
        .capes
        .into_iter()
        .map(|cape| CapeView {
            id: cape.id,
            alias: cape.alias,
            url: cape.url,
            active: cape.state == "ACTIVE",
        })
        .collect())
}

pub async fn set_cape(
    client: &reqwest::Client,
    account: &Account,
    cape_id: Option<&str>,
) -> Result<()> {
    if account.kind == "authlib" {
        bail!("第三方账号的披风需要在皮肤站切换");
    }
    if account.kind != "microsoft" {
        bail!("离线账号没有披风");
    }
    let refreshed = auth::refresh(client, account)
        .await
        .context("刷新微软登录凭据失败")?;
    let url = "https://api.minecraftservices.com/minecraft/profile/capes/active";
    let response = match cape_id {
        Some(id) => {
            client
                .put(url)
                .bearer_auth(&refreshed.access_token)
                .json(&serde_json::json!({ "capeId": id }))
                .send()
                .await?
        }
        None => {
            client
                .delete(url)
                .bearer_auth(&refreshed.access_token)
                .send()
                .await?
        }
    };
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let text = response.text().await.unwrap_or_default();
    let message = match status.as_u16() {
        401 => "登录凭据已失效，请重新登录微软账号".to_string(),
        429 => "操作过于频繁，请稍后再试".to_string(),
        _ => format!("披风设置失败（{status}）：{text}"),
    };
    bail!(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_of(width: u32, height: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&vec![0u8; (width * height * 4) as usize])
                .unwrap();
        }
        out
    }

    /// 披风只接受 64×32，皮肤那套 64×64 不能被当成披风混进来
    #[test]
    fn cape_png_must_be_64x32() {
        assert!(normalize_cape_png(png_of(64, 32)).is_ok());
        assert!(normalize_cape_png(png_of(64, 64)).is_err());
        assert!(normalize_cape_png(b"not a png".to_vec()).is_err());
    }

    /// 材质上传要退回站点根，不能把 authlib 的 /api/yggdrasil 前缀再拼一遍
    #[test]
    fn site_root_strips_authlib_prefix() {
        assert_eq!(
            site_root("https://littleskin.cn/api/yggdrasil"),
            "https://littleskin.cn"
        );
        assert_eq!(
            site_root("https://littleskin.cn/api/yggdrasil/"),
            "https://littleskin.cn"
        );
        assert_eq!(site_root("https://littleskin.cn/api"), "https://littleskin.cn");
        assert_eq!(
            site_root("https://skin.example.com"),
            "https://skin.example.com"
        );
    }

    /// 64×32 旧皮肤转 64×64 时帽子图层不能丢，否则头发/头盔消失，看起来像换了个皮肤
    #[test]
    fn legacy_skin_keeps_hat_layer() {
        let mut raw = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut raw, 64, 32);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            let mut data = vec![0u8; (64 * 32 * 4) as usize];
            let idx = ((64 + 33) * 4) as usize;
            data[idx..idx + 4].copy_from_slice(&[255, 0, 0, 255]);
            writer.write_image_data(&data).unwrap();
        }
        let normalized = normalize_png(raw).unwrap();
        let decoder = png::Decoder::new(std::io::Cursor::new(&normalized));
        let mut reader = decoder.read_info().unwrap();
        let mut buffer = vec![0; reader.output_buffer_size()];
        let frame = reader.next_frame(&mut buffer).unwrap();
        buffer.truncate(frame.buffer_size());
        let idx = ((64 + 33) * 4) as usize;
        assert_eq!(&buffer[idx..idx + 4], &[255, 0, 0, 255]);
    }

    /// 联网实测：能从官方档案解析出皮肤与模型。
    /// cargo test --lib skins::tests::live_official_skin -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_official_skin() {
        let client = crate::http::client();
        // jeb_ 的官方皮肤
        let (bytes, model) = official_skin(&client, "853c80ef3c3749fdaa49938b674adae6")
            .await
            .expect("读取皮肤失败");
        println!("皮肤 {} 字节，模型 {model}", bytes.len());
        assert!(!bytes.is_empty());
        assert_eq!(model, "classic");
    }
}
