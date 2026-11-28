use crate::config::{self, Config};
use crate::install;
use anyhow::{anyhow, bail, Context, Result};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

fn meta_path() -> PathBuf {
    config::data_dir().join("instances.json")
}

/// 每个版本独立的元数据与启动覆盖项，按版本 ID 存储。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct InstanceMeta {
    pub display_name: Option<String>,
    pub description: String,
    pub icon: String,
    pub category: String,
    pub favorite: bool,
    /// 该版本是否使用独立目录；整合包实例会强制开启
    pub isolated: Option<bool>,
    pub memory_mb: Option<u32>,
    pub memory_mode: Option<String>,
    pub java_path: Option<String>,
    pub java_args: Option<String>,
    pub game_args: Option<String>,
    pub pre_launch: Option<String>,
    pub disable_launch_wrapper: Option<bool>,
    pub disable_lwjgl_unsafe: Option<bool>,
    pub prefer_gpu: Option<bool>,
    pub window_width: Option<u32>,
    pub window_height: Option<u32>,
    pub fullscreen: Option<bool>,
}

fn load_map() -> HashMap<String, InstanceMeta> {
    std::fs::read(meta_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_map(map: &HashMap<String, InstanceMeta>) -> Result<()> {
    std::fs::create_dir_all(config::data_dir())?;
    std::fs::write(meta_path(), serde_json::to_vec_pretty(map)?)?;
    Ok(())
}

fn meta_key(cfg: &Config, version_id: &str) -> String {
    format!(
        "{}::{}",
        cfg.game_dir
            .trim_end_matches(['\\', '/'])
            .to_lowercase(),
        version_id
    )
}

pub fn get(cfg: &Config, version_id: &str) -> InstanceMeta {
    let map = load_map();
    map.get(&meta_key(cfg, version_id))
        .cloned()
        // 兼容旧版只按版本 ID 存储的数据
        .or_else(|| map.get(version_id).cloned())
        .unwrap_or_default()
}

pub fn save(cfg: &Config, version_id: &str, meta: &InstanceMeta) -> Result<()> {
    let mut map = load_map();
    map.insert(meta_key(cfg, version_id), meta.clone());
    save_map(&map)
}

pub fn remove(cfg: &Config, version_id: &str) -> Result<()> {
    let mut map = load_map();
    let mut changed = map.remove(&meta_key(cfg, version_id)).is_some();
    changed |= map.remove(version_id).is_some();
    if changed {
        save_map(&map)?;
    }
    remove_icon_images(version_id);
    Ok(())
}

/* ---------- 自定义图标 ---------- */

fn icon_dir() -> PathBuf {
    config::data_dir().join("instance_icons")
}

fn icon_stem(version_id: &str) -> String {
    version_id
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .take(64)
        .collect()
}

fn remove_icon_images(version_id: &str) {
    let stem = icon_stem(version_id);
    let Ok(entries) = std::fs::read_dir(icon_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .file_stem()
            .map(|value| value.to_string_lossy() == stem)
            .unwrap_or(false)
        {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// 只改图标字段，避免前端为了换图标重传整份设置
pub fn set_icon(cfg: &Config, version_id: &str, icon: &str) -> Result<()> {
    let mut meta = get(cfg, version_id);
    meta.icon = icon.to_string();
    save(cfg, version_id, &meta)
}

/// 把用户选的图片复制进数据目录并标记为自定义图标
pub fn set_icon_image(cfg: &Config, version_id: &str, source: &str) -> Result<String> {
    let src = Path::new(source);
    let ext = src
        .extension()
        .map(|value| value.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if !matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp") {
        bail!("图标只支持 png / jpg / webp / gif / bmp 图片");
    }
    std::fs::create_dir_all(icon_dir())?;
    remove_icon_images(version_id);
    let dest = icon_dir().join(format!("{}.{ext}", icon_stem(version_id)));
    std::fs::copy(src, &dest)
        .with_context(|| format!("复制图标失败：{}", src.display()))?;
    set_icon(cfg, version_id, "custom")?;
    Ok("custom".into())
}

/// 返回自定义图标的 data URL，前端直接放进 img
pub fn icon_image(cfg: &Config, version_id: &str) -> Option<String> {
    if get(cfg, version_id).icon != "custom" {
        return None;
    }
    let stem = icon_stem(version_id);
    let entries = std::fs::read_dir(icon_dir()).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .file_stem()
            .map(|value| value.to_string_lossy() == stem)
            .unwrap_or(false)
        {
            let bytes = std::fs::read(&path).ok()?;
            let mime = match path
                .extension()
                .map(|value| value.to_string_lossy().to_lowercase())
                .as_deref()
            {
                Some("png") => "image/png",
                Some("webp") => "image/webp",
                Some("gif") => "image/gif",
                Some("bmp") => "image/bmp",
                _ => "image/jpeg",
            };
            return Some(format!(
                "data:{mime};base64,{}",
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes)
            ));
        }
    }
    None
}

/// 只返回当前游戏目录下的版本元数据，前端按版本 ID 取用。
pub fn all(cfg: &Config) -> HashMap<String, InstanceMeta> {
    let prefix = format!(
        "{}::",
        cfg.game_dir
            .trim_end_matches(['\\', '/'])
            .to_lowercase()
    );
    let mut result = HashMap::new();
    for (key, meta) in load_map() {
        if let Some(version_id) = key.strip_prefix(&prefix) {
            result.insert(version_id.to_string(), meta);
        } else if !key.contains("::") {
            // 旧格式数据直接按 ID 兼容
            result.insert(key, meta);
        }
    }
    result
}

/// 把版本级覆盖项叠到全局配置上，供启动时使用。
pub fn apply(cfg: &Config, version_id: &str) -> Config {
    let meta = get(cfg, version_id);
    let mut merged = cfg.clone();
    if let Some(memory) = meta.memory_mb {
        merged.memory_mb = memory.max(512);
        merged.memory_auto = false;
    }
    if let Some(mode) = meta.memory_mode.filter(|mode| !mode.trim().is_empty()) {
        merged.memory_mode = mode;
    }
    if let Some(path) = meta.java_path.filter(|path| !path.trim().is_empty()) {
        merged.java_path = Some(path);
    }
    if let Some(args) = meta.java_args {
        merged.java_args = args;
    }
    if let Some(args) = meta.game_args {
        merged.game_args = args;
    }
    if let Some(command) = meta.pre_launch {
        merged.pre_launch = command;
    }
    if let Some(value) = meta.disable_launch_wrapper {
        merged.disable_launch_wrapper = value;
    }
    if let Some(value) = meta.disable_lwjgl_unsafe {
        merged.disable_lwjgl_unsafe = value;
    }
    if let Some(value) = meta.prefer_gpu {
        merged.prefer_gpu = value;
    }
    if let Some(width) = meta.window_width.filter(|value| *value >= 320) {
        merged.window_width = width;
    }
    if let Some(height) = meta.window_height.filter(|value| *value >= 240) {
        merged.window_height = height;
    }
    if let Some(fullscreen) = meta.fullscreen {
        merged.fullscreen = fullscreen;
    }
    merged
}

#[derive(Debug, Clone, Serialize)]
pub struct InstanceFile {
    pub name: String,
    pub display_name: String,
    pub size: u64,
    pub modified: u64,
    pub enabled: bool,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct InstanceCounts {
    pub mods: u32,
    pub resourcepacks: u32,
    pub shaderpacks: u32,
    pub saves: u32,
    pub screenshots: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstancePaths {
    pub version: String,
    pub mods: String,
    pub resourcepacks: String,
    pub shaderpacks: String,
    pub saves: String,
    pub screenshots: String,
    pub datapacks: String,
    pub schematics: String,
    pub game_dir: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorldSummary {
    /// 存档文件夹名，前端拼成 datapacks:<folder> 作为内容分类
    pub folder: String,
    pub name: String,
    pub modified: u64,
    pub datapacks: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorldSeed {
    pub folder: String,
    pub name: String,
    /// 世界种子，读不到时为 None
    pub seed: Option<String>,
    /// level.dat 里记录的世界版本，例如 1.21.1
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstanceInfo {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub icon: String,
    pub category: String,
    pub favorite: bool,
    pub isolated: Option<bool>,
    pub game_version: String,
    pub loader: String,
    /// 加载器版本号，非标准加载器目录时为空
    pub loader_version: String,
    pub kind: String,
    pub java_major: u32,
    pub path: String,
    pub installed_at: u64,
    pub memory_mb: Option<u32>,
    pub memory_mode: Option<String>,
    pub java_path: Option<String>,
    pub java_args: Option<String>,
    pub game_args: Option<String>,
    pub pre_launch: Option<String>,
    pub disable_launch_wrapper: Option<bool>,
    pub disable_lwjgl_unsafe: Option<bool>,
    pub prefer_gpu: Option<bool>,
    pub window_width: Option<u32>,
    pub window_height: Option<u32>,
    pub fullscreen: Option<bool>,
    pub counts: InstanceCounts,
}

/// 前端与下载源对同一分类的叫法并不统一（例如 Modrinth 用 shaders，目录叫 shaderpacks），
/// 这里统一收口，避免「未知的内容分类」把安装流程中断。
fn content_subdir(kind: &str) -> Result<&'static str> {
    match kind {
        "mods" | "mod" => Ok("mods"),
        "resourcepacks" | "resourcepack" | "texturepacks" => Ok("resourcepacks"),
        "shaderpacks" | "shaders" | "shader" => Ok("shaderpacks"),
        "saves" | "worlds" | "world" => Ok("saves"),
        "screenshots" | "screenshot" => Ok("screenshots"),
        "schematics" | "projection" | "litematica" => Ok("schematics"),
        _ => bail!("未知的内容分类：{kind}"),
    }
}

pub fn content_dir(cfg: &Config, version_id: &str, kind: &str) -> Result<PathBuf> {
    // 数据包属于单个存档世界：datapacks:<folder>，不带后缀时取最近游玩的世界
    if kind == "datapacks" {
        return datapack_dir(cfg, version_id, None);
    }
    if let Some(world) = kind.strip_prefix("datapacks:") {
        return datapack_dir(cfg, version_id, Some(world));
    }
    let dir = game_dir(cfg, version_id).join(content_subdir(kind)?);
    // 实例目录是懒创建的：首次安装这类内容时目录还不存在
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// 实例实际使用的游戏目录，整合包可以通过元数据强制独立。
pub fn game_dir(cfg: &Config, version_id: &str) -> PathBuf {
    let isolated = get(cfg, version_id)
        .isolated
        .unwrap_or(cfg.version_isolation);
    if isolated {
        cfg.game_root().join("versions").join(version_id)
    } else {
        cfg.game_root()
    }
}

fn latest_world_dir(cfg: &Config, version_id: &str) -> Result<PathBuf> {
    worlds(cfg, version_id)
        .into_iter()
        .next()
        .map(|world| game_dir(cfg, version_id).join("saves").join(world.folder))
        .ok_or_else(|| anyhow::anyhow!("还没有存档，请先在游戏里创建一个世界"))
}

fn datapack_dir(cfg: &Config, version_id: &str, world: Option<&str>) -> Result<PathBuf> {
    let world_dir = match world.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => {
            let name = safe_name(name)?;
            let dir = game_dir(cfg, version_id).join("saves").join(name);
            if !dir.is_dir() {
                bail!("存档不存在：{name}");
            }
            dir
        }
        None => latest_world_dir(cfg, version_id)?,
    };
    let target = world_dir.join("datapacks");
    std::fs::create_dir_all(&target)?;
    Ok(target)
}

/// 实例下的全部存档世界，按最近游玩时间倒序。
pub fn worlds(cfg: &Config, version_id: &str) -> Vec<WorldSummary> {
    let saves = game_dir(cfg, version_id).join("saves");
    let Ok(entries) = std::fs::read_dir(&saves) else {
        return Vec::new();
    };
    let mut list = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let folder = entry.file_name().to_string_lossy().to_string();
        if folder.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let mut name = folder.clone();
        if let Ok(text) = std::fs::read_to_string(path.join("levelname.txt")) {
            let trimmed = text.trim();
            if !trimmed.is_empty() && trimmed.chars().count() <= 64 {
                name = trimmed.to_string();
            }
        }
        list.push(WorldSummary {
            folder,
            name,
            modified: modified_secs(&path.join("level.dat")).max(modified_secs(&path)),
            datapacks: path.join("datapacks").to_string_lossy().to_string(),
        });
    }
    list.sort_by(|a, b| b.modified.cmp(&a.modified));
    list
}

/// 读取单个存档的种子与世界版本，用于种子地图预览。
pub fn world_seed(cfg: &Config, version_id: &str, folder: &str) -> Result<WorldSeed> {
    let name = safe_name(folder)?;
    let dir = game_dir(cfg, version_id).join("saves").join(name);
    if !dir.is_dir() {
        bail!("存档不存在：{name}");
    }
    let mut display = name.to_string();
    if let Ok(text) = std::fs::read_to_string(dir.join("levelname.txt")) {
        let trimmed = text.trim();
        if !trimmed.is_empty() && trimmed.chars().count() <= 64 {
            display = trimmed.to_string();
        }
    }
    let raw = std::fs::read(dir.join("level.dat")).context("读取 level.dat 失败")?;
    let data = if raw.starts_with(&[0x1f, 0x8b]) {
        let mut decoder = GzDecoder::new(raw.as_slice());
        let mut out = Vec::new();
        decoder
            .read_to_end(&mut out)
            .context("解压 level.dat 失败")?;
        out
    } else {
        raw
    };
    let (seed, version) = parse_level_dat(&data).unwrap_or((None, None));
    Ok(WorldSeed {
        folder: name.to_string(),
        name: display,
        seed: seed.map(|value| value.to_string()),
        version,
    })
}

struct LevelInfo {
    seed: Option<i64>,
    version: Option<String>,
}

/// 只解析 level.dat 根复合标签里的 Data.Seed / Data.Version.Name。
fn parse_level_dat(data: &[u8]) -> Option<(Option<i64>, Option<String>)> {
    let mut pos = 0usize;
    if *data.get(pos)? != 10 {
        return None;
    }
    pos += 1;
    read_nbt_string(data, &mut pos)?;
    let mut info = LevelInfo {
        seed: None,
        version: None,
    };
    scan_compound(data, &mut pos, &mut info, "root")?;
    Some((info.seed, info.version))
}

fn scan_compound(data: &[u8], pos: &mut usize, info: &mut LevelInfo, scope: &str) -> Option<()> {
    loop {
        let tag = *data.get(*pos)?;
        *pos += 1;
        if tag == 0 {
            return Some(());
        }
        let name = read_nbt_string(data, pos)?;
        match tag {
            10 if scope == "root" && name == "Data" => scan_compound(data, pos, info, "data")?,
            10 if scope == "data" && name == "Version" => scan_compound(data, pos, info, "version")?,
            10 if scope == "data" && name == "WorldGenSettings" => {
                scan_compound(data, pos, info, "worldgen")?
            }
            4 if scope == "data" && (name == "Seed" || name == "RandomSeed") => {
                let value = read_i64(data, pos)?;
                if info.seed.is_none() || name == "Seed" {
                    info.seed = Some(value);
                }
            }
            4 if scope == "worldgen" && name == "seed" => {
                info.seed = Some(read_i64(data, pos)?);
            }
            8 if scope == "version" && name == "Name" => {
                info.version = Some(read_nbt_string(data, pos)?);
            }
            _ => skip_nbt_payload(data, pos, tag)?,
        }
    }
}

fn read_nbt_string(data: &[u8], pos: &mut usize) -> Option<String> {
    let len = read_u16(data, pos)? as usize;
    let slice = data.get(*pos..*pos + len)?;
    *pos += len;
    Some(String::from_utf8_lossy(slice).to_string())
}

fn read_u16(data: &[u8], pos: &mut usize) -> Option<u16> {
    let bytes: [u8; 2] = data.get(*pos..*pos + 2)?.try_into().ok()?;
    *pos += 2;
    Some(u16::from_be_bytes(bytes))
}

fn read_i32(data: &[u8], pos: &mut usize) -> Option<i32> {
    let bytes: [u8; 4] = data.get(*pos..*pos + 4)?.try_into().ok()?;
    *pos += 4;
    Some(i32::from_be_bytes(bytes))
}

fn read_i64(data: &[u8], pos: &mut usize) -> Option<i64> {
    let bytes: [u8; 8] = data.get(*pos..*pos + 8)?.try_into().ok()?;
    *pos += 8;
    Some(i64::from_be_bytes(bytes))
}

fn skip_nbt_payload(data: &[u8], pos: &mut usize, tag: u8) -> Option<()> {
    match tag {
        1 => *pos += 1,
        2 => *pos += 2,
        3 => *pos += 4,
        4 => *pos += 8,
        5 => *pos += 4,
        6 => *pos += 8,
        7 => {
            let len = read_i32(data, pos)?.max(0) as usize;
            *pos += len;
        }
        8 => {
            read_nbt_string(data, pos)?;
        }
        9 => {
            let element = *data.get(*pos)?;
            *pos += 1;
            let len = read_i32(data, pos)?.max(0);
            for _ in 0..len {
                skip_nbt_payload(data, pos, element)?;
            }
        }
        10 => loop {
            let inner = *data.get(*pos)?;
            *pos += 1;
            if inner == 0 {
                break;
            }
            read_nbt_string(data, pos)?;
            skip_nbt_payload(data, pos, inner)?;
        },
        11 => {
            let len = read_i32(data, pos)?.max(0) as usize;
            *pos += len * 4;
        }
        12 => {
            let len = read_i32(data, pos)?.max(0) as usize;
            *pos += len * 8;
        }
        _ => return None,
    }
    data.get(*pos..*pos).map(|_| ())
}

pub fn paths(cfg: &Config, version_id: &str) -> InstancePaths {
    let root = game_dir(cfg, version_id);
    for name in [
        "mods",
        "resourcepacks",
        "shaderpacks",
        "saves",
        "screenshots",
        "schematics",
    ] {
        let _ = std::fs::create_dir_all(root.join(name));
    }
    InstancePaths {
        version: install::resolve_version_dir(cfg, version_id)
            .to_string_lossy()
            .to_string(),
        mods: root.join("mods").to_string_lossy().to_string(),
        resourcepacks: root.join("resourcepacks").to_string_lossy().to_string(),
        shaderpacks: root.join("shaderpacks").to_string_lossy().to_string(),
        saves: root.join("saves").to_string_lossy().to_string(),
        screenshots: root.join("screenshots").to_string_lossy().to_string(),
        datapacks: latest_world_dir(cfg, version_id)
            .map(|world| world.join("datapacks").to_string_lossy().to_string())
            .unwrap_or_default(),
        schematics: root.join("schematics").to_string_lossy().to_string(),
        game_dir: root.to_string_lossy().to_string(),
    }
}

fn modified_secs(path: &Path) -> u64 {
    std::fs::metadata(path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn dir_size(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| {
            let child = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => dir_size(&child),
                _ => entry.metadata().map(|meta| meta.len()).unwrap_or(0),
            }
        })
        .sum()
}

fn expand_rgba(src: &[u8], channels: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len() / channels * 4);
    for chunk in src.chunks_exact(channels) {
        match channels {
            1 => out.extend_from_slice(&[chunk[0], chunk[0], chunk[0], 255]),
            2 => out.extend_from_slice(&[chunk[0], chunk[0], chunk[0], chunk[1]]),
            3 => out.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 255]),
            _ => out.extend_from_slice(&chunk[..4]),
        }
    }
    out
}

/// 把 PNG 等比缩到最长边 max 以内，编码后转成 data URL；失败时返回 None。
fn png_thumb_data_url(path: &Path, max: u32) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    // 16 位图标不剥成 8 位的话，下面按每像素 4 字节展开会整张错位
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let (width, height) = (info.width, info.height);
    if width == 0 || height == 0 {
        return None;
    }
    let used = info.buffer_size();
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..used].to_vec(),
        png::ColorType::Rgb => expand_rgba(&buf[..used], 3),
        png::ColorType::Grayscale => expand_rgba(&buf[..used], 1),
        png::ColorType::GrayscaleAlpha => expand_rgba(&buf[..used], 2),
        png::ColorType::Indexed => return None,
    };

    let (tw, th) = if width <= max && height <= max {
        (width, height)
    } else {
        let scale = max as f64 / width.max(height) as f64;
        (
            ((width as f64 * scale).round() as u32).max(1),
            ((height as f64 * scale).round() as u32).max(1),
        )
    };

    let pixels = if tw == width && th == height {
        rgba
    } else {
        let mut out = vec![0u8; (tw * th * 4) as usize];
        for y in 0..th {
            let sy0 = (y as u64 * height as u64 / th as u64) as u32;
            let sy1 = (((y + 1) as u64 * height as u64 / th as u64) as u32).max(sy0 + 1).min(height);
            for x in 0..tw {
                let sx0 = (x as u64 * width as u64 / tw as u64) as u32;
                let sx1 = (((x + 1) as u64 * width as u64 / tw as u64) as u32).max(sx0 + 1).min(width);
                let mut sum = [0u32; 4];
                let mut count = 0u32;
                for sy in sy0..sy1 {
                    for sx in sx0..sx1 {
                        let i = ((sy * width + sx) * 4) as usize;
                        for c in 0..4 {
                            sum[c] += rgba[i + c] as u32;
                        }
                        count += 1;
                    }
                }
                let o = ((y * tw + x) * 4) as usize;
                let count = count.max(1);
                for c in 0..4 {
                    out[o + c] = (sum[c] / count) as u8;
                }
            }
        }
        out
    };

    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, tw, th);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&pixels).ok()?;
    }
    Some(crate::skins::to_data_url(&encoded, "image/png"))
}

pub fn list_files(cfg: &Config, version_id: &str, kind: &str) -> Result<Vec<InstanceFile>> {
    let dir = content_dir(cfg, version_id, kind)?;
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(Vec::new());
    };
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
        if kind == "saves" && !is_dir {
            continue;
        }
        let enabled = !name.ends_with(".disabled");
        let mut display_name = name.trim_end_matches(".disabled").to_string();
        let mut size = if is_dir {
            0
        } else {
            std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0)
        };
        match kind {
            "saves" => {
                // 世界文件夹的 levelname.txt 才是玩家看到的存档名
                if let Ok(text) = std::fs::read_to_string(path.join("levelname.txt")) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() && trimmed.chars().count() <= 64 {
                        display_name = trimmed.to_string();
                    }
                }
                size = dir_size(&path);
            }
            _ => {}
        }
        files.push(InstanceFile {
            name,
            display_name,
            size,
            modified: modified_secs(&path),
            enabled,
            is_dir,
        });
    }
    files.sort_by(|a, b| b.enabled.cmp(&a.enabled).then(a.display_name.cmp(&b.display_name)));
    Ok(files)
}

/// 可编辑的文本后缀：贴图、声音等二进制文件不列出来，避免 config 树被资源文件淹没
const CONFIG_TEXT_EXT: &[&str] = &[
    "toml", "json", "json5", "cfg", "conf", "config", "properties", "txt", "yml", "yaml",
    "ini", "mcmeta", "snbt", "lang", "hjson", "xml", "csv", "tsv", "md",
];
/// 单个配置文件最多 2 MB，再大基本是数据文件，塞进编辑器只会卡死界面
const CONFIG_MAX_BYTES: u64 = 2 * 1024 * 1024;
const CONFIG_MAX_ENTRIES: usize = 3000;
const CONFIG_MAX_DEPTH: usize = 8;

#[derive(Debug, Clone, Serialize)]
pub struct ConfigEntry {
    /// 相对 config 目录的路径，统一用 `/`
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigText {
    pub content: String,
    /// "utf-8" 或 "gbk"：写回时按原编码保存
    pub encoding: String,
    pub size: u64,
}

fn config_root(cfg: &Config, version_id: &str) -> Result<PathBuf> {
    let dir = game_dir(cfg, version_id).join("config");
    if !dir.is_dir() {
        bail!("这个实例还没有 config 目录，先启动一次游戏再来");
    }
    Ok(dir)
}

fn is_config_text(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| CONFIG_TEXT_EXT.contains(&ext.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// 列出 config 目录下的文本文件（含子目录），限制深度与条数防止超大整合包卡死。
pub fn config_tree(cfg: &Config, version_id: &str) -> Result<Vec<ConfigEntry>> {
    fn walk(dir: &Path, prefix: &str, depth: usize, out: &mut Vec<ConfigEntry>) {
        if depth > CONFIG_MAX_DEPTH || out.len() >= CONFIG_MAX_ENTRIES {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut items: Vec<_> = entries.flatten().collect();
        // 目录在前、文件在后，同级按名字排序
        items.sort_by_key(|entry| {
            let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
            (!is_dir, entry.file_name())
        });
        for entry in items {
            if out.len() >= CONFIG_MAX_ENTRIES {
                return;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            // 不跟随符号链接，避免把 config 之外的目录带进来
            if kind.is_symlink() {
                continue;
            }
            let rel = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            if kind.is_dir() {
                out.push(ConfigEntry {
                    path: rel.clone(),
                    name,
                    is_dir: true,
                    size: 0,
                });
                walk(&entry.path(), &rel, depth + 1, out);
            } else if kind.is_file() && is_config_text(&name) {
                let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
                out.push(ConfigEntry {
                    path: rel,
                    name,
                    is_dir: false,
                    size,
                });
            }
        }
    }

    let root = config_root(cfg, version_id)?;
    let mut out = Vec::new();
    walk(&root, "", 0, &mut out);
    Ok(out)
}

/// 把前端传来的相对路径解析成 config 目录内的真实文件，拒绝越界与目录。
fn config_file(cfg: &Config, version_id: &str, rel: &str) -> Result<PathBuf> {
    let root = config_root(cfg, version_id)?;
    let rel_path = Path::new(rel);
    if rel.trim().is_empty() || rel_path.is_absolute() {
        bail!("无效的文件路径");
    }
    if rel_path
        .components()
        .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        bail!("无效的文件路径");
    }
    let root = std::fs::canonicalize(&root).context("config 目录不可访问")?;
    let path = std::fs::canonicalize(root.join(rel_path)).context("文件不存在")?;
    if !path.starts_with(&root) {
        bail!("文件不在 config 目录内");
    }
    if !path.is_file() {
        bail!("不是可编辑的文件");
    }
    Ok(path)
}

/// 老配置常用系统 ANSI 代码页（简体中文下即 GBK）保存，UTF-8 解不开时用它兜底。
#[cfg(windows)]
fn decode_ansi(bytes: &[u8]) -> Option<String> {
    use windows_sys::Win32::Globalization::{MultiByteToWideChar, CP_ACP};
    unsafe {
        let len = MultiByteToWideChar(
            CP_ACP,
            0,
            bytes.as_ptr(),
            bytes.len() as i32,
            std::ptr::null_mut(),
            0,
        );
        if len <= 0 {
            return None;
        }
        let mut buf = vec![0u16; len as usize];
        let written = MultiByteToWideChar(
            CP_ACP,
            0,
            bytes.as_ptr(),
            bytes.len() as i32,
            buf.as_mut_ptr(),
            len,
        );
        if written <= 0 {
            return None;
        }
        String::from_utf16(&buf[..written as usize]).ok()
    }
}

#[cfg(not(windows))]
fn decode_ansi(_bytes: &[u8]) -> Option<String> {
    None
}

#[cfg(windows)]
fn encode_ansi(text: &str) -> Option<Vec<u8>> {
    use windows_sys::Win32::Globalization::{WideCharToMultiByte, CP_ACP};
    let wide: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        let len = WideCharToMultiByte(
            CP_ACP,
            0,
            wide.as_ptr(),
            wide.len() as i32,
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
            std::ptr::null_mut(),
        );
        if len <= 0 {
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        let written = WideCharToMultiByte(
            CP_ACP,
            0,
            wide.as_ptr(),
            wide.len() as i32,
            buf.as_mut_ptr(),
            len,
            std::ptr::null(),
            std::ptr::null_mut(),
        );
        if written <= 0 {
            return None;
        }
        buf.truncate(written as usize);
        Some(buf)
    }
}

#[cfg(not(windows))]
fn encode_ansi(_text: &str) -> Option<Vec<u8>> {
    None
}

pub fn read_config_text(cfg: &Config, version_id: &str, rel: &str) -> Result<ConfigText> {
    let path = config_file(cfg, version_id, rel)?;
    let meta = std::fs::metadata(&path)?;
    if meta.len() > CONFIG_MAX_BYTES {
        bail!(
            "文件有 {:.1} MB，超过 2 MB 的配置不在启动器内编辑，请用外部编辑器打开",
            meta.len() as f64 / 1024.0 / 1024.0
        );
    }
    let mut bytes = std::fs::read(&path)?;
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        bytes.drain(..3);
    }
    let size = bytes.len() as u64;
    let (content, encoding) = match String::from_utf8(bytes.clone()) {
        Ok(text) => (text, "utf-8"),
        Err(_) => match decode_ansi(&bytes) {
            Some(text) => (text, "gbk"),
            None => bail!("这个文件不是可识别的文本编码，请用外部编辑器打开"),
        },
    };
    Ok(ConfigText {
        content,
        encoding: encoding.to_string(),
        size,
    })
}

pub fn write_config_text(
    cfg: &Config,
    version_id: &str,
    rel: &str,
    content: &str,
    encoding: &str,
) -> Result<()> {
    if content.len() as u64 > CONFIG_MAX_BYTES {
        bail!("内容超过 2 MB，无法在启动器内保存");
    }
    let path = config_file(cfg, version_id, rel)?;
    let bytes = if encoding.eq_ignore_ascii_case("gbk") {
        let encoded = encode_ansi(content).ok_or_else(|| anyhow!("按 GBK 编码保存失败"))?;
        // 往返校验：GBK 表示不了的字符会被静默替换成 '?'，这里宁可报错也不悄悄改坏配置
        if decode_ansi(&encoded).as_deref() != Some(content) {
            bail!("内容里有 GBK 无法表示的字符（例如 emoji），请改用 UTF-8 或删除这些字符");
        }
        encoded
    } else {
        content.as_bytes().to_vec()
    };
    std::fs::write(&path, bytes).with_context(|| format!("写入失败：{}", path.display()))?;
    Ok(())
}

fn safe_name(name: &str) -> Result<&str> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name == ".." {
        bail!("无效的文件名");
    }
    Ok(name)
}

pub fn toggle_file(cfg: &Config, version_id: &str, kind: &str, name: &str) -> Result<()> {
    if !matches!(kind, "mods" | "resourcepacks" | "shaderpacks") {
        bail!("该分类不支持启用/禁用");
    }
    let name = safe_name(name)?;
    let dir = content_dir(cfg, version_id, kind)?;
    let path = dir.join(name);
    if !path.exists() {
        bail!("文件不存在：{name}");
    }
    let target = if name.ends_with(".disabled") {
        dir.join(name.trim_end_matches(".disabled"))
    } else {
        dir.join(format!("{name}.disabled"))
    };
    if target.exists() {
        bail!("目标文件已存在");
    }
    std::fs::rename(&path, &target).context("重命名失败，游戏可能正在运行")?;
    Ok(())
}

pub fn delete_file(cfg: &Config, version_id: &str, kind: &str, name: &str) -> Result<()> {
    let name = safe_name(name)?;
    let dir = content_dir(cfg, version_id, kind)?;
    let path = dir.join(name);
    if !path.exists() {
        bail!("文件不存在：{name}");
    }
    if path.is_dir() {
        std::fs::remove_dir_all(&path)?;
    } else {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

/// 按需生成缩略图：截图返回缩放后的 data URL，存档返回世界图标。
pub fn thumb(cfg: &Config, version_id: &str, kind: &str, name: &str) -> Result<Option<String>> {
    let name = safe_name(name)?;
    let dir = content_dir(cfg, version_id, kind)?;
    let path = dir.join(name);
    if !path.exists() {
        return Ok(None);
    }
    Ok(match kind {
        "screenshots" => png_thumb_data_url(&path, 480),
        "saves" => png_thumb_data_url(&path.join("icon.png"), 128)
            .or_else(|| png_thumb_data_url(&path.join("level.png"), 128)),
        _ => None,
    })
}

/// 用系统默认程序打开实例内容目录里的文件（截图查看用）。
pub fn open_content(cfg: &Config, version_id: &str, kind: &str, name: &str) -> Result<()> {
    let name = safe_name(name)?;
    let dir = content_dir(cfg, version_id, kind)?;
    let path = dir.join(name);
    if !path.exists() {
        bail!("文件不存在：{name}");
    }
    #[cfg(windows)]
    {
        std::process::Command::new("explorer")
            .arg(&path)
            .spawn()
            .context("打开文件失败")?;
        return Ok(());
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .context("打开文件失败")?;
        Ok(())
    }
}

fn unique_target(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    let extension = path.extension().map(|value| value.to_string_lossy().to_string());
    for index in 1..1000 {
        let next = match &extension {
            Some(ext) => format!("{stem} ({index}).{ext}"),
            None => format!("{stem} ({index})"),
        };
        let candidate = dir.join(next);
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(name)
}

fn copy_dir_all(src: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)?.flatten() {
        let path = entry.path();
        let target = dest.join(entry.file_name());
        if path.is_dir() {
            copy_dir_all(&path, &target)?;
        } else {
            std::fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

pub fn import_paths(
    cfg: &Config,
    version_id: &str,
    kind: &str,
    paths: &[String],
) -> Result<u32> {
    let dir = content_dir(cfg, version_id, kind)?;
    std::fs::create_dir_all(&dir)?;
    let mut count = 0;
    for raw in paths {
        let src = PathBuf::from(raw);
        if !src.exists() {
            continue;
        }
        let name = src
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let target = unique_target(&dir, &name);
        if src.is_dir() {
            copy_dir_all(&src, &target)?;
        } else {
            std::fs::copy(&src, &target)?;
        }
        count += 1;
    }
    Ok(count)
}

pub fn info(cfg: &Config, version_id: &str) -> Result<InstanceInfo> {
    let meta = get(cfg, version_id);
    let (game_version, loader) = install::instance_context(cfg, version_id)?;
    let merged = install::resolve_detail(cfg, version_id)?;
    let installed = install::list_installed(cfg);
    let entry = installed
        .iter()
        .find(|item| item.id == version_id)
        .ok_or_else(|| anyhow::anyhow!("版本未安装：{version_id}"))?;

    let counts = InstanceCounts {
        mods: list_files(cfg, version_id, "mods")
            .map(|list| list.len() as u32)
            .unwrap_or(0),
        resourcepacks: list_files(cfg, version_id, "resourcepacks")
            .map(|list| list.len() as u32)
            .unwrap_or(0),
        shaderpacks: list_files(cfg, version_id, "shaderpacks")
            .map(|list| list.len() as u32)
            .unwrap_or(0),
        saves: list_files(cfg, version_id, "saves")
            .map(|list| list.len() as u32)
            .unwrap_or(0),
        screenshots: list_files(cfg, version_id, "screenshots")
            .map(|list| list.len() as u32)
            .unwrap_or(0),
    };

    Ok(InstanceInfo {
        id: version_id.to_string(),
        display_name: meta
            .display_name
            .clone()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| version_id.to_string()),
        description: meta.description.clone(),
        icon: if meta.icon.is_empty() {
            "auto".into()
        } else {
            meta.icon.clone()
        },
        category: if meta.category.is_empty() {
            "auto".into()
        } else {
            meta.category.clone()
        },
        favorite: meta.favorite,
        isolated: meta.isolated,
        game_version: game_version.clone(),
        loader: loader.clone(),
        loader_version: install::loader_version_from_detail(&merged.detail, &loader, &game_version)
            .or_else(|| install::loader_version_from_id(version_id, &loader, &game_version))
            .unwrap_or_default(),
        kind: merged.detail.kind.clone(),
        java_major: merged.detail.required_java(),
        path: install::resolve_version_dir(cfg, version_id)
            .to_string_lossy()
            .to_string(),
        installed_at: entry.last_modified,
        memory_mb: meta.memory_mb,
        memory_mode: meta.memory_mode,
        java_path: meta.java_path,
        java_args: meta.java_args,
        game_args: meta.game_args,
        pre_launch: meta.pre_launch,
        disable_launch_wrapper: meta.disable_launch_wrapper,
        disable_lwjgl_unsafe: meta.disable_lwjgl_unsafe,
        prefer_gpu: meta.prefer_gpu,
        window_width: meta.window_width,
        window_height: meta.window_height,
        fullscreen: meta.fullscreen,
        counts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(version_id: &str) -> Config {
        let root = std::env::temp_dir().join(format!(
            "docl-config-test-{}-{version_id}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("versions").join(version_id).join("config")).unwrap();
        let cfg = Config {
            game_dir: root.to_string_lossy().to_string(),
            version_isolation: true,
            ..Default::default()
        };
        cfg
    }

    #[test]
    fn config_text_roundtrip_keeps_encoding() {
        let id = "cfg-roundtrip";
        let cfg = test_config(id);
        let root = Path::new(&cfg.game_dir).join("versions").join(id);
        std::fs::write(
            root.join("config").join("jei.toml"),
            "[general]\n# 中文注释\nmode = true\n",
        )
        .unwrap();

        let tree = config_tree(&cfg, id).unwrap();
        assert!(tree
            .iter()
            .any(|entry| entry.path == "jei.toml" && !entry.is_dir));

        let file = read_config_text(&cfg, id, "jei.toml").unwrap();
        assert_eq!(file.encoding, "utf-8");
        assert!(file.content.contains("中文注释"));

        let edited = "[general]\n# 改过的注释\nmode = false\n";
        write_config_text(&cfg, id, "jei.toml", edited, "utf-8").unwrap();
        assert_eq!(read_config_text(&cfg, id, "jei.toml").unwrap().content, edited);

        let _ = std::fs::remove_dir_all(&cfg.game_dir);
    }

    #[cfg(windows)]
    #[test]
    fn config_text_handles_gbk_files() {
        let id = "cfg-gbk";
        let cfg = test_config(id);
        let root = Path::new(&cfg.game_dir).join("versions").join(id);
        let raw = encode_ansi("# 中文注释\n").unwrap();
        std::fs::write(root.join("config").join("legacy.cfg"), raw).unwrap();

        let file = read_config_text(&cfg, id, "legacy.cfg").unwrap();
        assert_eq!(file.encoding, "gbk");
        assert!(file.content.contains("中文注释"));

        write_config_text(&cfg, id, "legacy.cfg", "# 改过的注释\n", "gbk").unwrap();
        let again = read_config_text(&cfg, id, "legacy.cfg").unwrap();
        assert_eq!(again.encoding, "gbk");
        assert!(again.content.contains("改过的注释"));

        let _ = std::fs::remove_dir_all(&cfg.game_dir);
    }

    #[test]
    fn config_text_rejects_paths_outside_config() {
        let id = "cfg-escape";
        let cfg = test_config(id);
        let root = Path::new(&cfg.game_dir).join("versions").join(id);
        std::fs::write(root.join("secret.txt"), "outside").unwrap();

        assert!(read_config_text(&cfg, id, "../secret.txt").is_err());
        assert!(write_config_text(&cfg, id, "../secret.txt", "x", "utf-8").is_err());
        assert!(read_config_text(&cfg, id, "C:/Windows/win.ini").is_err());

        let _ = std::fs::remove_dir_all(&cfg.game_dir);
    }

    /// 16 位 PNG 图标要先剥成 8 位再展开，否则缩略图会整张错位或直接生成失败
    #[test]
    fn png_thumb_handles_16bit_image() {
        use base64::Engine;

        let dir = std::env::temp_dir().join(format!("docl-thumb-16bit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("icon.png");

        let mut raw = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut raw, 2, 2);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Sixteen);
            let mut writer = encoder.write_header().unwrap();
            // 2×2 纯红（16 位大端）
            let mut data = Vec::new();
            for _ in 0..4 {
                data.extend_from_slice(&[0xff, 0xff, 0x00, 0x00, 0x00, 0x00]);
            }
            writer.write_image_data(&data).unwrap();
        }
        std::fs::write(&path, raw).unwrap();

        let url = png_thumb_data_url(&path, 32).expect("缩略图生成失败");
        let payload = url.split_once(',').map(|(_, rest)| rest).unwrap_or("");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(payload)
            .unwrap();
        let decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
        let mut reader = decoder.read_info().unwrap();
        let mut buffer = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buffer).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgba);
        assert_eq!((info.width, info.height), (2, 2));
        assert_eq!(&buffer[0..4], &[255, 0, 0, 255]);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
