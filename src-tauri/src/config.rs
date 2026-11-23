use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GameDirEntry {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServerEntry {
    pub address: String,
    pub last_used: u64,
}

/// 用户创建的主题：只存关键色，其余颜色在应用时按它们派生
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CustomTheme {
    pub id: String,
    pub name: String,
    /// 页面底色
    pub paper: String,
    /// 卡片底色
    pub panel: String,
    /// 主色（按钮、链接）
    pub primary: String,
    /// 强调色（选中态、进度条）
    pub accent: String,
    /// 标题文字
    pub title: String,
    /// 正文文字
    pub text: String,
    /// 次要文字
    pub muted: String,
    /// 边框线
    pub line: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// 当前使用的游戏根目录（含 versions / assets / libraries）
    pub game_dir: String,
    /// 保存过的游戏目录，可在设置里切换
    pub game_dirs: Vec<GameDirEntry>,
    /// 最近连接过的服务器
    pub recent_servers: Vec<ServerEntry>,
    /// 房间大厅数据地址（返回 JSON 列表），留空表示不启用大厅
    pub hall_url: String,
    /// 附加的 EasyTier 公共节点（逗号或空格分隔），开房/加入时传给陶瓦
    pub terracotta_nodes: String,
    /// 自更新清单地址，留空表示不检查更新
    pub update_url: String,
    /// 启动器更新策略：auto（自动下载）/ notify（提示）/ off（关闭）
    pub update_mode: String,
    /// 手动指定的 java.exe
    pub java_path: Option<String>,
    /// 手动添加过的 Java 列表（自动检测不到的自定义安装），退出后仍保留
    pub extra_java_paths: Vec<String>,
    /// 启动器下载 Java 时的安装根目录，留空表示用内置 runtime 目录
    pub java_install_dir: Option<String>,
    /// 最大内存（MB）
    pub memory_mb: u32,
    /// 自动按系统内存推荐分配
    pub memory_auto: bool,
    /// auto / g1gc / zgc / custom
    pub memory_mode: String,
    /// 额外 JVM 参数
    pub java_args: String,
    /// 额外游戏参数
    pub game_args: String,
    /// 启动前执行的命令
    pub pre_launch: String,
    /// 不再向游戏注入 DOCL 启动器标识
    pub disable_launch_wrapper: bool,
    /// 不再注入 Java 24+ 的 LWJGL Unsafe 兼容参数
    pub disable_lwjgl_unsafe: bool,
    /// 为所选 Java 写入高性能显卡偏好
    pub prefer_gpu: bool,
    pub window_width: u32,
    pub window_height: u32,
    pub fullscreen: bool,
    /// auto / official / mirror
    pub download_source: String,
    pub concurrency: u32,
    /// 全局下载限速（MiB/s），0 表示不限速
    pub speed_limit: u32,
    pub animations: bool,
    pub version_isolation: bool,
    pub selected_version: Option<String>,
    pub selected_account: Option<String>,
    /// paper / celadon / terracotta / moon / sakura / ocean / lavender / graphite /
    /// forge / ink / neon / launcher
    pub theme: String,
    pub background_image: Option<String>,
    /// 背景图文件夹：设置后每次启动从中随机选一张（与 background_image 互斥）
    pub background_dir: Option<String>,
    /// 背景适应方式：cover（铺满裁剪）/ contain（完整显示）/ stretch（拉伸）/ center（原始大小）
    pub background_fit: String,
    pub background_blur: u32,
    pub glass_opacity: u32,
    pub glass_blur: u32,
    /// 背景遮罩暗度（0-60）
    pub background_dim: u32,
    /// 版本页「游玩档案」卡片底图，空表示用内置图
    #[serde(default)]
    pub profile_hero_image: Option<String>,
    /// 版本页「最常玩版本」卡片底图，空表示用内置图
    #[serde(default)]
    pub profile_card_image: Option<String>,
    /// 版本页两张卡片的文字颜色，空表示跟随主题
    #[serde(default)]
    pub profile_text_color: Option<String>,
    /// 自定义主题列表；theme 为 `custom:<id>` 时启用对应主题
    #[serde(default)]
    pub custom_themes: Vec<CustomTheme>,
    /// default / sharp / round
    pub ui_radius: String,
    /// default / yahei / songti / kaiti
    pub ui_font: String,
    /// compact / normal / comfy
    pub ui_density: String,
    /// Windows Acrylic：窗口毛玻璃化，透出桌面实时画面
    pub window_vibrancy: bool,
    /// 低内存模式：降低 WebView2 缓存与 JS 堆上限，并关闭毛玻璃/动效（重启后生效）
    pub low_memory: bool,
    /// 从 MC 百科补全模组中文名（会访问 search.mcmod.cn，已做节流与缓存）
    pub mcmod_names: bool,
    /// 记住启动器窗口大小：关闭时保存，下次打开恢复
    #[serde(default = "default_true")]
    pub remember_window: bool,
    /// 上次关闭时的窗口尺寸（逻辑像素），0 表示还没记录过
    #[serde(default)]
    pub launcher_window_w: u32,
    #[serde(default)]
    pub launcher_window_h: u32,
    /// 上次保存时便携版的根目录（exe 所在目录）。
    /// 便携包被剪走/换盘/改名后，加载时据此把旧绝对路径重写到新位置。
    #[serde(default)]
    pub portable_root: Option<String>,
}

fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            game_dir: default_game_dir().to_string_lossy().to_string(),
            game_dirs: Vec::new(),
            recent_servers: Vec::new(),
            hall_url: String::new(),
            terracotta_nodes: String::new(),
            // 默认走国内加速；玩家可在设置里切到 GitHub 原地址
            update_url: "https://cdn.jsdelivr.net/gh/dawo4/DOCL@main/update.json".into(),
            update_mode: "notify".into(),
            java_path: None,
            extra_java_paths: Vec::new(),
            java_install_dir: None,
            memory_mb: 4096,
            memory_auto: true,
            memory_mode: "auto".into(),
            java_args: String::new(),
            game_args: String::new(),
            pre_launch: String::new(),
            disable_launch_wrapper: false,
            disable_lwjgl_unsafe: false,
            prefer_gpu: false,
            window_width: 1280,
            window_height: 720,
            fullscreen: false,
            download_source: "auto".into(),
            // 32 是安装资源（几千个小文件）与日常下载的平衡点；
            // 全局并发闸门直接按这个值封顶
            concurrency: 32,
            speed_limit: 0,
            animations: true,
            version_isolation: true,
            selected_version: None,
            selected_account: None,
            theme: "paper".into(),
            background_image: Some("builtin:forest".into()),
            background_dir: None,
            background_fit: "cover".into(),
            background_blur: 4,
            glass_opacity: 62,
            glass_blur: 18,
            background_dim: 0,
            profile_hero_image: None,
            profile_card_image: None,
            profile_text_color: None,
            custom_themes: Vec::new(),
            ui_radius: "default".into(),
            ui_font: "default".into(),
            ui_density: "normal".into(),
            // 便携版默认透出桌面：配合外带场景的玻璃质感；安装版保持实心
            window_vibrancy: is_portable(),
            low_memory: false,
            mcmod_names: true,
            remember_window: true,
            launcher_window_w: 0,
            launcher_window_h: 0,
            portable_root: None,
        }
    }
}

/// 便携版标记：exe 同目录存在 portable.txt 或 data 目录时启用。
/// 只读位置（Program Files、只读介质）无法写入时自动回退到用户目录，避免启动即报错。
fn portable_root() -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let data = dir.join("data");
    let marked = dir.join("portable.txt").is_file() || data.is_dir();
    if !marked {
        return None;
    }
    fs::create_dir_all(&data).ok()?;
    Some(dir)
}

fn portable_base() -> Option<&'static PathBuf> {
    static CACHE: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    CACHE.get_or_init(portable_root).as_ref()
}

/// 当前是否为便携版（配置、账号、实例索引都放在 exe 同目录的 data 下）。
pub fn is_portable() -> bool {
    portable_base().is_some()
}

pub fn data_dir() -> PathBuf {
    if let Some(root) = portable_base() {
        return root.join("data");
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("McLauncher")
}

pub fn runtime_dir() -> PathBuf {
    data_dir().join("runtime")
}

pub fn default_game_dir() -> PathBuf {
    if let Some(root) = portable_base() {
        return root.join("data").join(".minecraft");
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".minecraft")
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.json")
}

/// 便携版的锚点：exe 所在目录。非便携版返回 None。
fn portable_anchor() -> Option<String> {
    portable_base().map(|root| {
        root.to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .to_string()
    })
}

/// value 是否位于 root 之下；是则返回去掉根之后的部分（大小写不敏感）。
fn strip_root_ci(value: &str, root: &str) -> Option<String> {
    let value = value.trim();
    let root = root.trim_end_matches(['\\', '/']);
    if root.is_empty() || value.len() < root.len() {
        return None;
    }
    let head = value.get(..root.len())?;
    if !head.eq_ignore_ascii_case(root) {
        return None;
    }
    let rest = &value[root.len()..];
    if rest.is_empty() {
        return Some(String::new());
    }
    // 必须是真正的子路径，避免 D:\DOCL2 误命中 D:\DOCL
    if !rest.starts_with(['\\', '/']) {
        return None;
    }
    Some(rest.trim_start_matches(['\\', '/']).to_string())
}

fn rebase_path(value: &str, old_root: &str, new_root: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.starts_with("builtin:") {
        return None;
    }
    let rest = strip_root_ci(value, old_root)?;
    let mut out = new_root.trim_end_matches(['\\', '/']).to_string();
    if !rest.is_empty() {
        out.push('\\');
        out.push_str(&rest);
    }
    Some(out)
}


fn dir_label(path: &str) -> String {
    let trimmed = path.trim_end_matches(['\\', '/']);
    std::path::Path::new(trimmed)
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| path.to_string())
}

fn same_path(a: &str, b: &str) -> bool {
    a.trim_end_matches(['\\', '/']).eq_ignore_ascii_case(b.trim_end_matches(['\\', '/']))
}

pub fn accounts_path() -> PathBuf {
    data_dir().join("accounts.json")
}

pub fn load() -> Config {
    let mut config: Config = fs::read(config_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    // 整包搬走后先把旧根目录下的路径重写到新位置，再校验不可用的路径
    config.rebase_paths();
    config.heal_paths();
    config.sync_game_dirs();
    config.portable_root = portable_anchor();
    config
}

pub fn save(cfg: &Config) -> Result<()> {
    // 锚点永远以当前 exe 位置为准，避免把别处的路径带进配置文件
    let mut snapshot = cfg.clone();
    snapshot.portable_root = portable_anchor();
    fs::create_dir_all(data_dir())?;
    fs::write(config_path(), serde_json::to_vec_pretty(&snapshot)?)?;
    Ok(())
}

impl Config {
    pub fn game_root(&self) -> PathBuf {
        PathBuf::from(&self.game_dir)
    }

    /// 便携包换位置后，把以上次的根目录开头的绝对路径改写到当前根目录。
    pub fn rebase_paths(&mut self) {
        let Some(new_root) = portable_anchor() else {
            return;
        };
        let Some(old_root) = self.portable_root.clone() else {
            return;
        };
        if old_root.trim().is_empty() || same_path(&old_root, &new_root) {
            return;
        }
        let rebase = |value: &str| rebase_path(value, &old_root, &new_root);
        if let Some(next) = rebase(&self.game_dir) {
            self.game_dir = next;
        }
        for entry in &mut self.game_dirs {
            if let Some(next) = rebase(&entry.path) {
                entry.path = next;
            }
        }
        if let Some(next) = self.java_path.as_deref().and_then(rebase) {
            self.java_path = Some(next);
        }
        self.extra_java_paths = self
            .extra_java_paths
            .iter()
            .map(|path| rebase(path).unwrap_or_else(|| path.clone()))
            .collect();
        if let Some(next) = self.java_install_dir.as_deref().and_then(rebase) {
            self.java_install_dir = Some(next);
        }
        for slot in [
            &mut self.background_image,
            &mut self.background_dir,
            &mut self.profile_hero_image,
            &mut self.profile_card_image,
        ] {
            if let Some(next) = slot.as_deref().and_then(&rebase) {
                *slot = Some(next);
            }
        }
    }

    /// 校验关键路径：游戏目录失效时回到默认位置，保证「解压就玩」；
    /// 其余的失效路径回落到空值，让启动器重新自动选择。
    pub fn heal_paths(&mut self) {
        let game_ok = !self.game_dir.trim().is_empty()
            && std::path::Path::new(self.game_dir.trim()).is_dir();
        if !game_ok {
            self.game_dir = default_game_dir().to_string_lossy().to_string();
        }
        if let Some(dir) = self.java_install_dir.clone() {
            let dir = dir.trim().to_string();
            if dir.is_empty() || !std::path::Path::new(&dir).is_dir() {
                self.java_install_dir = None;
            }
        }
        if let Some(path) = self.java_path.clone() {
            if !std::path::Path::new(path.trim()).is_file() {
                self.java_path = None;
            }
        }
        self.extra_java_paths
            .retain(|path| std::path::Path::new(path.trim()).is_file());
        for slot in [
            &mut self.background_image,
            &mut self.background_dir,
            &mut self.profile_hero_image,
            &mut self.profile_card_image,
        ] {
            let Some(value) = slot.clone() else { continue };
            let value = value.trim().to_string();
            if value.starts_with("builtin:") || std::path::Path::new(&value).exists() {
                continue;
            }
            // 路径整体失效时按文件名在 data 目录里找回，搬家后背景图还在
            let recovered = std::path::Path::new(&value)
                .file_name()
                .map(|name| data_dir().join(name))
                .filter(|path| path.exists());
            *slot = recovered.map(|path| path.to_string_lossy().to_string());
        }
    }

    /// 确保目录列表里包含当前目录，并给缺失名称的条目补上默认名字。
    pub fn sync_game_dirs(&mut self) {
        self.game_dirs.retain(|entry| !entry.path.trim().is_empty());
        for entry in self.game_dirs.iter_mut() {
            if entry.name.trim().is_empty() {
                entry.name = dir_label(&entry.path);
            }
        }
        if !self
            .game_dirs
            .iter()
            .any(|entry| same_path(&entry.path, &self.game_dir))
        {
            self.game_dirs.insert(
                0,
                GameDirEntry {
                    name: dir_label(&self.game_dir),
                    path: self.game_dir.clone(),
                },
            );
        }
    }

    pub fn set_active_dir(&mut self, path: &str) {
        self.game_dir = path.to_string();
        self.sync_game_dirs();
    }

    /// 记住一个手动添加的 Java：去重后记入额外列表，并设为优先使用。
    /// 自动检测不到的安装（自定义目录、绿色版 JDK）靠这份列表长期保留。
    pub fn remember_java(&mut self, path: &str) {
        let path = path.trim();
        if path.is_empty() {
            return;
        }
        if !self
            .extra_java_paths
            .iter()
            .any(|item| item.eq_ignore_ascii_case(path))
        {
            self.extra_java_paths.push(path.to_string());
        }
        self.java_path = Some(path.to_string());
    }

    /// 把一个目录加入列表（已存在则跳过），不切换当前目录。
    /// 返回是否真的新增，方便调用方统计。
    pub fn add_dir(&mut self, path: &str) -> bool {
        let path = path.trim();
        if path.is_empty() {
            return false;
        }
        if self.game_dirs.iter().any(|entry| same_path(&entry.path, path)) {
            return false;
        }
        self.game_dirs.push(GameDirEntry {
            name: dir_label(path),
            path: path.to_string(),
        });
        true
    }

    pub fn use_mirror(&self) -> bool {
        self.download_source != "official"
    }

    /// Java 运行时的根目录：优先用用户指定的路径，否则回落到内置 runtime。
    pub fn java_runtime_root(&self) -> PathBuf {
        match self.java_install_dir.as_deref().map(str::trim) {
            Some(dir) if !dir.is_empty() => PathBuf::from(dir),
            _ => runtime_dir(),
        }
    }

    pub fn launch_dir(&self, version_id: &str) -> PathBuf {
        if self.version_isolation {
            self.game_root().join("versions").join(version_id)
        } else {
            self.game_root()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{default_game_dir, rebase_path, Config, CustomTheme};

    fn theme() -> CustomTheme {
        CustomTheme {
            id: "theme-test".into(),
            name: "夜航".into(),
            paper: "#101820".into(),
            panel: "#182430".into(),
            primary: "#3e8c8c".into(),
            accent: "#e0a458".into(),
            title: "#eef3f7".into(),
            text: "#cfd8e0".into(),
            muted: "#8a99a8".into(),
            line: "#2c3a46".into(),
        }
    }

    /// 自定义主题要能完整存取，否则重启后主题会丢
    #[test]
    fn custom_themes_survive_roundtrip() {
        let mut cfg = Config::default();
        cfg.custom_themes.push(theme());
        cfg.theme = "custom:theme-test".into();
        let json = serde_json::to_string(&cfg).unwrap();
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back.custom_themes, vec![theme()]);
        assert_eq!(back.theme, "custom:theme-test");
    }

    /// 旧配置文件没有 custom_themes 字段时按空列表加载，不能整个读失败
    #[test]
    fn old_config_without_custom_themes_still_loads() {
        let mut value = serde_json::to_value(Config::default()).unwrap();
        value.as_object_mut().unwrap().remove("custom_themes");
        let cfg: Config = serde_json::from_value(value).unwrap();
        assert!(cfg.custom_themes.is_empty());
    }

    /// 多次手动添加 Java 要全部保留，且大小写不同的同一路径不能重复
    #[test]
    fn manual_java_paths_accumulate() {
        let mut cfg = Config::default();
        cfg.remember_java(r"C:\Java\jdk17\bin\java.exe");
        cfg.remember_java(r"D:\jdks\jdk21\bin\java.exe");
        cfg.remember_java(r"c:\java\JDK17\bin\java.exe");
        assert_eq!(cfg.extra_java_paths.len(), 2);
        assert_eq!(cfg.extra_java_paths[0], r"C:\Java\jdk17\bin\java.exe");
        assert_eq!(cfg.java_path.as_deref(), Some(r"c:\java\JDK17\bin\java.exe"));
    }

    /// 旧配置没有 extra_java_paths 字段时也要能加载
    #[test]
    fn old_config_without_extra_java_paths_still_loads() {
        let mut value = serde_json::to_value(Config::default()).unwrap();
        value.as_object_mut().unwrap().remove("extra_java_paths");
        let cfg: Config = serde_json::from_value(value).unwrap();
        assert!(cfg.extra_java_paths.is_empty());
    }

    /// 整包搬家：旧根目录下的路径要改成新根，且不能误伤前缀相同的兄弟目录
    #[test]
    fn rebases_paths_between_roots() {
        let old = r"C:\Users\me\Desktop\DOCL";
        let new = r"D:\Games\DOCL";
        assert_eq!(
            rebase_path(r"C:\Users\me\Desktop\DOCL\data\.minecraft", old, new).as_deref(),
            Some(r"D:\Games\DOCL\data\.minecraft")
        );
        // 大小写不敏感
        assert_eq!(
            rebase_path(r"c:\users\ME\desktop\docl\data\bg.png", old, new).as_deref(),
            Some(r"D:\Games\DOCL\data\bg.png")
        );
        // 前缀相同但不是子路径，不能替换
        assert!(rebase_path(r"C:\Users\me\Desktop\DOCL2\data", old, new).is_none());
        // 外部路径、内置背景保持原样
        assert!(rebase_path(r"E:\Other\.minecraft", old, new).is_none());
        assert!(rebase_path("builtin:forest", old, new).is_none());
    }

    /// 游戏目录不存在时回落到默认位置，存在的自定义目录保持不变
    #[test]
    fn heals_missing_game_dir() {
        let mut cfg = Config::default();
        cfg.game_dir = r"Z:\nowhere\.minecraft".into();
        cfg.heal_paths();
        assert_eq!(cfg.game_dir, default_game_dir().to_string_lossy());

        let existing = std::env::temp_dir().join("docl-heal-test");
        std::fs::create_dir_all(&existing).unwrap();
        cfg.game_dir = existing.to_string_lossy().to_string();
        cfg.heal_paths();
        assert_eq!(cfg.game_dir, existing.to_string_lossy());
        let _ = std::fs::remove_dir_all(&existing);
    }
}
