use crate::config::Config;
use crate::instance;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};
use std::time::UNIX_EPOCH;

/// 小文件整读；大文件只保留头部与尾部（崩溃堆栈在尾部）。
const WHOLE_LIMIT: u64 = 6 * 1024 * 1024;
const HEAD_LIMIT: u64 = 512 * 1024;
const TAIL_LIMIT: u64 = 4 * 1024 * 1024;
/// 崩溃报告与 latest.log 的写入时间差在这个范围内时，认为崩溃报告属于本次运行。
const SAME_RUN_WINDOW: u64 = 90;

#[derive(Debug, Clone, Serialize)]
pub struct CrashLink {
    pub label: String,
    pub url: String,
}

/// 可疑 Mod：来自堆栈包名或崩溃报告的 Suspected Mods。
#[derive(Debug, Clone, Serialize)]
pub struct CrashSuspect {
    pub name: String,
    /// mods 目录里的文件名，未匹配到时为空
    pub file_name: String,
    /// 命中的包名（堆栈分析时）
    pub package: String,
    /// stack / reported
    pub reason: String,
}

/// 分析时读取到的实例上下文，用于生成更具体的建议。
#[derive(Debug, Clone, Serialize, Default)]
pub struct CrashContext {
    pub game_version: String,
    pub loader: String,
    pub java_major: u32,
    pub java_path: String,
    pub memory_mb: u32,
    pub mod_count: u32,
    /// hs_err 的 Problematic frame
    pub problematic_frame: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CrashMatch {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub advice: Vec<String>,
    pub evidence: Vec<String>,
    pub links: Vec<CrashLink>,
    /// 90 高（基本可直接定位）/ 70 中 / 50 低
    pub priority: u8,
    /// 1 环境/安装 → 2 Mod 归因 → 3 兼容 → 4 兜底；只展示最早一层的命中
    pub phase: u8,
    /// high / medium / low
    pub confidence: String,
    /// java / memory / mod / loader / graphics / file / world / server / auth
    pub category: String,
    pub suspects: Vec<CrashSuspect>,
}

/// 自上次成功启动以来的 Mod 文件变化。
/// Windows 应用程序日志里与本次崩溃时间接近的条目，用于识别进程被系统结束等情况。
#[derive(Debug, Clone, Serialize)]
pub struct WindowsCrashEvent {
    pub event_id: u32,
    pub provider: String,
    pub time_created: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModChange {
    /// added / removed / modified
    pub kind: String,
    pub file_name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CrashReport {
    pub version_id: String,
    pub found: bool,
    pub source: String,
    pub source_path: String,
    pub source_kind: String,
    pub modified: u64,
    pub headline: String,
    pub description: Option<String>,
    pub matches: Vec<CrashMatch>,
    pub excerpt: Vec<String>,
    pub scanned: Vec<String>,
    pub suspects: Vec<CrashSuspect>,
    pub mod_changes: Vec<ModChange>,
    pub windows_events: Vec<WindowsCrashEvent>,
    pub context: CrashContext,
}

struct Rule {
    id: &'static str,
    title: &'static str,
    summary: &'static str,
    patterns: &'static [&'static str],
    advice: &'static [&'static str],
    links: &'static [(&'static str, &'static str)],
    priority: u8,
    category: &'static str,
}

/// 远端规则库地址。规则只做文本匹配，不包含任何可执行内容。
const RULES_URL: &str = "https://cdn.jsdelivr.net/gh/dawo4/DOCL@main/crash-rules.json";
/// 编译期打包一份规则，首次离线启动时也能用上完整规则集
const BUNDLED_RULES: &str = include_str!("../../crash-rules.json");
/// 防止远端配置异常拖慢分析
const MAX_REMOTE_RULES: usize = 200;
const MAX_PATTERNS_PER_RULE: usize = 40;

#[derive(Debug, Clone, Deserialize)]
struct RemoteLink {
    label: String,
    url: String,
}

#[derive(Debug, Clone, Deserialize)]
struct RemoteRule {
    id: String,
    title: String,
    #[serde(default)]
    summary: String,
    patterns: Vec<String>,
    #[serde(default)]
    advice: Vec<String>,
    #[serde(default)]
    links: Vec<RemoteLink>,
    #[serde(default = "default_priority")]
    priority: u8,
    #[serde(default)]
    category: String,
    /// any（默认）：任一 pattern 命中；all：所有 pattern 都要命中
    #[serde(default = "default_match", rename = "match")]
    match_mode: String,
    /// 规则所在层：1 环境/安装、2 Mod 归因、3 兼容、4 兜底；缺省按 id 推断
    #[serde(default)]
    phase: Option<u8>,
    /// high / medium / low；缺省按 phase 推断
    #[serde(default)]
    confidence: Option<String>,
}

fn default_priority() -> u8 {
    50
}

fn default_match() -> String {
    "any".into()
}

#[derive(Debug, Deserialize)]
struct RemoteRuleFile {
    #[serde(default)]
    #[allow(dead_code)]
    rules_version: u32,
    rules: Vec<RemoteRule>,
}

/// 内置规则编译在二进制里兜底，远端只做增量覆盖：
/// 远端文件损坏、为空或离线时，已有规则照常工作。
static REMOTE_RULES: OnceLock<RwLock<Vec<RemoteRule>>> = OnceLock::new();

fn remote_store() -> &'static RwLock<Vec<RemoteRule>> {
    REMOTE_RULES.get_or_init(|| RwLock::new(Vec::new()))
}

fn remote_rules() -> Vec<RemoteRule> {
    remote_store()
        .read()
        .map(|guard| guard.clone())
        .unwrap_or_default()
}

/// 规则是否命中：any 只需要一条 pattern，all 要求全部出现。
fn rule_matches(rule: &RemoteRule, lowered: &str) -> bool {
    if rule.patterns.is_empty() {
        return false;
    }
    if rule.match_mode == "all" {
        rule.patterns
            .iter()
            .all(|pattern| lowered.contains(pattern.as_str()))
    } else {
        rule.patterns
            .iter()
            .any(|pattern| lowered.contains(pattern.as_str()))
    }
}

fn rules_cache_path() -> PathBuf {
    crate::config::data_dir().join("crash-rules.json")
}

/// 上一次成功更新的规则副本，用于回滚。
fn rules_prev_path() -> PathBuf {
    crate::config::data_dir().join("crash-rules.prev.json")
}

/// 被用户回滚掉的规则摘要：远端下发同一版时保持回滚结果，不再自动覆盖。
fn rules_skip_path() -> PathBuf {
    crate::config::data_dir().join("crash-rules.skip")
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// 清洗远端规则：丢掉缺关键字段的条目，pattern 统一小写，并限制数量。
fn sanitize(rules: Vec<RemoteRule>) -> Vec<RemoteRule> {
    let mut rules: Vec<RemoteRule> = rules
        .into_iter()
        .filter(|rule| {
            !rule.id.trim().is_empty()
                && !rule.title.trim().is_empty()
                && !rule.patterns.is_empty()
        })
        .map(|mut rule| {
            rule.patterns = rule
                .patterns
                .iter()
                .map(|pattern| pattern.trim().to_lowercase())
                .filter(|pattern| !pattern.is_empty())
                .take(MAX_PATTERNS_PER_RULE)
                .collect();
            // 只认 all，其余一律按 any 处理，避免远端写错字段导致规则永不命中
            rule.match_mode = if rule.match_mode.trim().eq_ignore_ascii_case("all") {
                "all".into()
            } else {
                "any".into()
            };
            rule
        })
        .filter(|rule| !rule.patterns.is_empty())
        .collect();
    rules.truncate(MAX_REMOTE_RULES);
    rules
}

/// 启动时先读本地缓存；没有缓存或缓存损坏时用编译期打包的规则兜底。
pub fn load_cached_rules() {
    let file = fs::read_to_string(rules_cache_path())
        .ok()
        .filter(|text| !text.trim().is_empty())
        .and_then(|text| serde_json::from_str::<RemoteRuleFile>(&text).ok())
        .or_else(|| serde_json::from_str::<RemoteRuleFile>(BUNDLED_RULES).ok());
    let Some(file) = file else {
        return;
    };
    if let Ok(mut guard) = remote_store().write() {
        *guard = sanitize(file.rules);
    }
}

/// 拉取最新规则并写入缓存。失败时保留现有规则，不影响崩溃分析。
pub async fn refresh_rules(client: &reqwest::Client) -> Result<usize> {
    let response = client.get(RULES_URL).send().await?;
    if !response.status().is_success() {
        anyhow::bail!("规则库返回 HTTP {}", response.status());
    }
    let bytes = response.bytes().await?;
    let file: RemoteRuleFile = serde_json::from_slice(&bytes)?;
    let rules = sanitize(file.rules);
    if rules.is_empty() {
        anyhow::bail!("规则库内容为空");
    }
    // 用户回滚过这一版：内容没变就保持回滚后的规则，等远端真正更新再接受
    if let Ok(skip) = fs::read_to_string(rules_skip_path()) {
        if skip.trim() == sha256_hex(&bytes) {
            return Ok(remote_store()
                .read()
                .map(|guard| guard.len())
                .unwrap_or(0));
        }
    }
    let count = rules.len();
    if let Ok(mut guard) = remote_store().write() {
        *guard = rules;
    }
    if fs::create_dir_all(crate::config::data_dir()).is_ok() {
        let cache = rules_cache_path();
        // 内容确实变了才留一份旧副本，避免每次启动都把上一版覆盖成同一份
        if let Ok(previous) = fs::read(&cache) {
            if previous != bytes.as_ref() {
                let _ = fs::write(rules_prev_path(), previous);
            }
        }
        let _ = fs::write(cache, &bytes);
        // 远端已经换了一版，之前的回滚标记不再需要
        let _ = fs::remove_file(rules_skip_path());
    }
    Ok(count)
}

/// 当前生效的规则来源与条数。
#[derive(Debug, Serialize)]
pub struct RuleStatus {
    /// bundled（内置）/ cache（缓存副本）
    pub source: String,
    pub count: usize,
    /// 缓存更新时间（Unix 秒），0 表示使用内置规则
    pub updated: u64,
    /// 是否可以回滚到上一版
    pub has_previous: bool,
}

pub fn rules_status() -> RuleStatus {
    let count = remote_store()
        .read()
        .map(|guard| guard.len())
        .unwrap_or(0);
    let cache = rules_cache_path();
    // 缓存文件存在但解析失败时实际用的是内置规则，不能标成「远端更新」
    let cache_ok = fs::read_to_string(&cache)
        .ok()
        .and_then(|text| serde_json::from_str::<RemoteRuleFile>(&text).ok())
        .is_some();
    let updated = cache_ok
        .then(|| {
            fs::metadata(&cache)
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs())
        })
        .flatten()
        .unwrap_or(0);
    let has_previous = fs::read_to_string(rules_prev_path())
        .ok()
        .and_then(|text| serde_json::from_str::<RemoteRuleFile>(&text).ok())
        .map(|file| !sanitize(file.rules).is_empty())
        .unwrap_or(false);
    RuleStatus {
        source: if cache_ok { "cache" } else { "bundled" }.into(),
        count,
        updated,
        has_previous,
    }
}

/// 回滚到上一版规则；回滚本身也可以再撤回一次。
pub fn rollback_rules() -> Result<usize> {
    let cache = rules_cache_path();
    let rejected = fs::read(&cache).ok();
    let (rules, _) = swap_rule_files(&cache, &rules_prev_path())?;
    let count = rules.len();
    if let Ok(mut guard) = remote_store().write() {
        *guard = rules;
    }
    // 记住被拒绝的这一版，下次启动拉取到同样内容时不再自动覆盖回滚结果
    match rejected {
        Some(bytes) => {
            let _ = fs::write(rules_skip_path(), sha256_hex(&bytes));
        }
        None => {
            let _ = fs::remove_file(rules_skip_path());
        }
    }
    Ok(count)
}

/// 把上一版规则换成当前规则，并把原当前版本留作下一次回滚目标。
fn swap_rule_files(cache: &Path, prev: &Path) -> Result<(Vec<RemoteRule>, Vec<u8>)> {
    let bytes = fs::read(prev).map_err(|_| anyhow::anyhow!("没有可回滚的规则版本"))?;
    let file: RemoteRuleFile = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("上一版规则文件已损坏，无法回滚"))?;
    let rules = sanitize(file.rules);
    if rules.is_empty() {
        bail!("上一版规则内容为空，已取消回滚");
    }
    let current = fs::read(cache).ok();
    if let Some(parent) = cache.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(cache, &bytes)?;
    match current {
        Some(text) => fs::write(prev, text)?,
        None => {
            let _ = fs::remove_file(prev);
        }
    }
    Ok((rules, bytes))
}

const RULES: &[Rule] = &[
    Rule {
        id: "java-version",
        title: "Java 版本不匹配",
        summary: "当前 Java 无法加载游戏或加载器的类文件，通常是 Java 版本低于该版本的要求。",
        patterns: &[
            "unsupportedclassversionerror",
            "class file major version",
            "has been compiled by a more recent version of the java runtime",
        ],
        advice: &[
            "在「设置 → Java」里安装或选择该游戏版本要求的 Java，版本号以诊断里的「当前实例」为准。",
            "版本设置里如果手动指定过 java.exe，确认它没有指向旧版。",
            "改完后重新启动游戏。",
        ],
        links: &[("下载 Temurin JDK", "https://adoptium.net/temurin/releases/")],
        priority: 90,
        category: "java",
    },
    Rule {
        id: "out-of-memory",
        title: "内存不足",
        summary: "Java 堆内存不够，常见于大型整合包、高清材质或视距过高。",
        patterns: &[
            "java.lang.outofmemoryerror",
            "gc overhead limit exceeded",
            "java heap space",
            "could not reserve enough space",
            "unable to allocate",
            "outofmemoryerror",
        ],
        advice: &[
            "在设置或版本设置里调大内存，整合包建议 6–8 GB，但不要超过物理内存的一半。",
            "关闭浏览器等占用内存较多的程序。",
            "降低渲染距离、模拟距离或光影质量后重试。",
            "如果已经分到 8 GB 仍不足，检查是否使用了 32 位 Java。",
        ],
        links: &[],
        priority: 90,
        category: "memory",
    },
    Rule {
        id: "vm-options",
        title: "Java 启动参数无效",
        summary: "JVM 不认识设置的启动参数，或无法创建虚拟机，通常是自定义参数写错或内存过大。",
        patterns: &[
            "unrecognized vm option",
            "could not create the java virtual machine",
            "error: could not create",
            "invalid maximum heap size",
            "error occurred during initialization of vm",
        ],
        advice: &[
            "在设置或版本设置里清空自定义 Java 参数后重试。",
            "检查最大内存是否超过物理内存，或是否写成了非法数值。",
            "确认使用的 Java 版本支持这些参数（部分参数仅特定版本可用）。",
        ],
        links: &[],
        priority: 88,
        category: "java",
    },
    Rule {
        id: "duplicate-classpath",
        title: "启动库重复",
        summary: "版本清单里同一个库出现了两次，NeoForge 引导器构建文件系统时会直接报 Duplicate key 并退出。",
        patterns: &[
            "duplicate key",
            "cpw.mods.niofs.union",
            "jars-in-classpath",
        ],
        advice: &[
            "更新到最新版 DOCL 后重新启动：新版已按文件路径去重，不会再崩。",
            "或者打开 versions 下该版本的 json，删掉重复的 library 条目。",
            "这种清单通常由其他启动器生成，导入别人做好的整合包时比较常见。",
        ],
        links: &[],
        priority: 92,
        category: "loader",
    },
    Rule {
        id: "customskinloader-version-detect",
        title: "CustomSkinLoader 无法识别游戏版本",
        summary: "CustomSkinLoader 读不到 classpath 上的 version.json，把游戏当成旧版本，补丁没有命中后直接崩溃。",
        patterns: &[
            "customskinloader:skin-manager-patch",
            "customskinloader.ignorepatchfailure",
        ],
        advice: &[
            "更新到最新版 DOCL 后重新启动：新版会为 Forge / NeoForge 实例补上只含 version.json 的元数据包。",
            "检查 mods 里是否同时放了多个 CustomSkinLoader jar：Universal 与 ForgeV3 / Fabric 只能保留一个。",
            "临时应急可以在 Java 参数里加 -Dcustomskinloader.ignorePatchFailure=true，但皮肤不会加载。",
        ],
        links: &[("CustomSkinLoader 项目页", "https://modrinth.com/mod/customskinloader")],
        priority: 96,
        category: "mod",
    },
    Rule {
        id: "loader-incomplete",
        title: "加载器没有完整安装",
        summary: "Java 找不到加载器的引导类或启动 jar，通常是加载器安装中断、文件被删、或版本清单损坏。",
        patterns: &[
            "could not find or load main class cpw.mods.bootstraplauncher",
            "could not find or load main class cpw.mods.modlauncher",
            "could not find or load main class net.fabricmc",
            "error: could not find or load main class",
            "unable to access jarfile",
        ],
        advice: &[
            "在版本设置里点「重装加载器」后重新启动；「一键修复」也会补回缺失的启动 jar 和依赖库。",
            "确认 versions 目录里该版本的 json 与 jar 没有被其他启动器或清理软件删掉。",
            "如果只是换了 Java，请确认选择的是与该加载器匹配的 Java 版本。",
        ],
        links: &[],
        priority: 90,
        category: "loader",
    },
    Rule {
        id: "module-package-conflict",
        title: "Mod 模块包冲突",
        summary: "两个模块把同一个包导出给同一个 Mod，常见于 Mod 内置了原版类，或皮肤/前置 Mod 装错了加载器分支。",
        patterns: &[
            "java.lang.module.resolutionexception",
            "export package",
            "reads another module named",
        ],
        advice: &[
            "看日志里「Modules A and minecraft export package ...」中的 A，A 通常就是冲突 Mod；优先把它更新到与当前游戏和加载器匹配的版本。",
            "CustomSkinLoader 要选对分支：1.17 及以上使用 ForgeV2 / ForgeActive，ForgeV1 或 Legacy 只用于旧版本。",
            "如果暂时不需要该 Mod，可在版本设置里禁用后启动；DOCL 已内置第三方皮肤同步，不装 CustomSkinLoader 也能显示皮肤。",
        ],
        links: &[],
        priority: 90,
        category: "mod",
    },
    Rule {
        id: "missing-dependency",
        title: "缺少前置 Mod 或类",
        summary: "某个 Mod 依赖的前置没有安装、版本不满足，或类在运行时找不到。",
        patterns: &[
            "missing or unsupported mandatory dependencies",
            "missing required dependency",
            "modresolutionexception",
            "missing dependency",
            "requires any version of",
            "requires version",
            "mod file is missing",
            "java.lang.noclassdeffounderror",
            // 只保留 Error：Mixin 里大量 "Error loading class: ... ClassNotFoundException"
            // 是可选目标的正常告警，按 Exception 匹配会误报成缺前置
            "java.lang.classnotfounderror",
        ],
        advice: &[
            "按日志里列出的 modid 安装对应前置，版本需与游戏、加载器一致。",
            "在本启动器的「下载 → 模组」里打开该 Mod 详情，前置区会列出必需依赖。",
            "整合包建议使用导入功能完整安装，避免手动拷贝漏掉依赖。",
        ],
        links: &[],
        priority: 86,
        category: "mod",
    },
    Rule {
        id: "mod-conflict",
        title: "Mod 冲突或加载失败",
        summary: "多个 Mod 互相冲突，或加载器无法解析依赖关系。",
        patterns: &[
            "duplicate mods found",
            "duplicate mod",
            "mod resolution failed",
            "incompatible mod set",
            "mod loading has failed",
            "modloadingexception",
            "loadingfailedexception",
        ],
        advice: &[
            "先禁用最近新增的 Mod，再逐个开启，定位冲突源。",
            "检查是否装了功能重复的 Mod（例如两个优化 Mod、两个同一前置的不同版本）。",
            "在崩溃报告里搜索「Failure message」，通常会直接写出冲突的 Mod ID。",
        ],
        links: &[],
        priority: 86,
        category: "mod",
    },
    Rule {
        id: "mixin",
        title: "Mixin 注入失败",
        summary: "某个 Mod 的 Mixin 补丁无法应用，通常是 Mod 版本与游戏或加载器不匹配。",
        patterns: &[
            "mixin apply failed",
            "mixintransformererror",
            "mixin.injection",
            "mixin config",
            "org.spongepowered.asm.mixin",
            "critical injection failure",
            "invalid mixin",
        ],
        advice: &[
            "确认该 Mod 支持当前游戏版本和加载器版本，必要时更新到匹配构建。",
            "如果同时装了多个优化 Mod（如 OptiFine 与 Sodium），只保留其中一个。",
            "在版本设置里临时禁用可疑 Mod 验证是否恢复。",
        ],
        links: &[],
        priority: 82,
        category: "mod",
    },
    Rule {
        id: "mod-version-mismatch",
        title: "Mod 与游戏版本不匹配",
        summary: "Mod 声明的 Minecraft / 加载器版本和当前实例不一致。",
        patterns: &[
            "requires minecraft",
            "requires forge",
            "requires fabric",
            "requires neoforge",
            "incompatible mods found",
            "mod requires",
            "unsupported minecraft version",
        ],
        advice: &[
            "按报错提示把 Mod 换成对应 Minecraft 与加载器版本的构建。",
            "在「版本设置 → 模组」里检查最近更新过的 Mod。",
            "整合包请通过启动器完整安装，避免手动替换单个 Mod。",
        ],
        links: &[],
        priority: 80,
        category: "mod",
    },
    Rule {
        id: "corrupted-jar",
        title: "文件损坏",
        summary: "某个 jar 下载不完整或已损坏，加载时无法解压。",
        patterns: &[
            "zip end header not found",
            "invalid or corrupted jar",
            "java.util.zip.zipexception",
            "error in opening zip file",
            "invalid jar file",
        ],
        advice: &[
            "在版本设置里重新下载该版本或对应的 Mod。",
            "检查 mods 目录里是否有 0 字节或异常的 jar 并删除。",
            "确认磁盘剩余空间充足。",
        ],
        links: &[],
        priority: 76,
        category: "file",
    },
    Rule {
        id: "class-loading",
        title: "类加载失败",
        summary: "Mod 与游戏版本不匹配，或缺少前置导致类初始化失败。",
        patterns: &[
            "java.lang.nosuchmethoderror",
            "java.lang.exceptioninitializererror",
            "java.lang.nosuchfielderror",
        ],
        advice: &[
            "确认所有 Mod 都是同一游戏版本。",
            "删除只适用于其他版本的 Mod。",
            "补齐报错里出现的类所属的前置 Mod。",
        ],
        links: &[],
        priority: 72,
        category: "mod",
    },
    Rule {
        id: "file-locked",
        title: "文件被占用",
        summary: "游戏文件正被另一个进程使用，常见于同时开了多个实例、上次游戏没完全退出，或安全软件正在扫描。",
        patterns: &[
            "being used by another process",
            "another instance",
            "already running",
            "access is denied",
            "failed to delete",
        ],
        advice: &[
            "关闭其他正在运行的游戏或启动器实例后重试，必要时在任务管理器里结束残留的 java.exe。",
            "如果杀毒软件拦截了文件读写，把游戏目录加入白名单。",
            "仍无法解决时重启电脑，再检查该文件是否被设为只读。",
        ],
        links: &[],
        priority: 70,
        category: "file",
    },
    Rule {
        id: "graphics",
        title: "显卡驱动 / OpenGL 问题",
        summary: "游戏无法创建渲染窗口或初始化 OpenGL，通常与显卡驱动有关。",
        patterns: &[
            "glfw error",
            "failed to create window",
            "failed to create glfw window",
            "pixel format not accelerated",
            "pixel format",
            "doesn't support opengl",
            "could not create opengl context",
            "opengl error",
            "wglcreatecontext",
            "unsupported graphics",
        ],
        advice: &[
            "到显卡厂商官网更新驱动，不要只依赖系统自带驱动。",
            "确认不是通过远程桌面或虚拟机运行游戏。",
            "在设置里打开「优先使用高性能显卡」后重试。",
        ],
        links: &[],
        priority: 70,
        category: "graphics",
    },
    Rule {
        id: "jvm-fatal",
        title: "Java 虚拟机崩溃",
        summary: "JVM 层发生严重错误（SIGSEGV/SIGILL 等），常见于驱动、本地库或内存不稳定。",
        patterns: &[
            "a fatal error has been detected by the java runtime environment",
            "sigtrap",
            "sigsegv",
            "hs_err_pid",
        ],
        advice: &[
            "更新显卡驱动和 Java 版本。",
            "关闭 CPU/内存超频（包括 XMP），检查内存稳定性。",
            "崩溃日志开头的 Problematic frame 会指向出问题的模块。",
        ],
        links: &[("崩溃报告说明", "https://minecraft.wiki/w/Crash_report")],
        priority: 66,
        category: "java",
    },
    Rule {
        id: "world-corrupt",
        title: "存档或区块损坏",
        summary: "读取世界数据时出错，可能是区块文件损坏或存档被其他进程占用。",
        patterns: &[
            "chunk file",
            "region file",
            "exception generating new chunk",
            "world corrupt",
            "failed to load world",
            "level.dat",
        ],
        advice: &[
            "先备份 saves 目录下的存档，再尝试进入。",
            "删除该世界的 session.lock 后重试（确保没有其他客户端开着它）。",
            "如果只有特定区域崩溃，可用区块修复工具处理，或从备份恢复。",
        ],
        links: &[],
        priority: 58,
        category: "world",
    },
    Rule {
        id: "server-port",
        title: "端口被占用",
        summary: "服务端启动时无法绑定端口，通常是已有实例或其他程序占用了同一端口。",
        patterns: &[
            "address already in use",
            "failed to bind to port",
            "bindexception",
        ],
        advice: &[
            "在联机页换一个端口启动，或先关闭已在运行的服务端。",
            "用 netstat 找到占用端口的程序后结束它。",
            "同一台机器同时开多个服务端时必须使用不同端口。",
        ],
        links: &[],
        priority: 56,
        category: "server",
    },
    Rule {
        id: "auth",
        title: "账号验证失败",
        summary: "正版登录状态失效，或无法连接 Mojang / Microsoft 的验证服务器。",
        patterns: &[
            "failed to verify username",
            "invalid session",
            "authentication servers",
            "invalid credentials",
        ],
        advice: &[
            "在账号页重新登录微软账号。",
            "检查网络或代理是否拦截了验证域名。",
            "离线账号请使用离线模式，或关闭服务器的正版验证。",
        ],
        links: &[],
        priority: 52,
        category: "auth",
    },
    Rule {
        id: "authlib-login",
        title: "第三方账号认证失败",
        summary: "皮肤站的登录凭据已过期，或游戏无法连接到该皮肤站。",
        // 只认真正的认证错误：authlib-injector 每次启动都会打印 INFO 日志，
        // 不能因为日志里出现了它，就把任意崩溃都判成第三方认证失败
        patterns: &[
            "failed to authenticate",
            "authentication failed",
            "unable to authenticate",
            "failed to fetch metadata",
        ],
        advice: &[
            "到账号页对第三方账号重新登录一次。",
            "确认皮肤站地址可用，且该服务器允许第三方认证。",
            "如果游戏内被踢并提示验证失败，说明进的是正版验证服务器。",
        ],
        links: &[],
        priority: 50,
        category: "auth",
    },
    /* ---------- 1.8.0 崩溃日志扩展 ---------- */
    Rule {
        id: "manual-debug-crash",
        title: "手动触发的调试崩溃",
        summary: "这次崩溃是按 F3+C 主动生成的调试报告，不是游戏或 Mod 故障。",
        patterns: &["manually triggered debug crash"],
        advice: &[
            "重启游戏即可，不需要修复或删除任何文件。",
            "如果是不小心误触，可以忽略这份崩溃报告。",
        ],
        links: &[],
        priority: 99,
        category: "other",
    },
    Rule {
        id: "confirmed-mod-crash",
        title: "已定位到具体 Mod",
        summary: "加载器直接点名了崩溃的 Mod，通常带有 “Caught exception from ...” 或 Failure message。",
        patterns: &[
            "caught exception from ",
            "loaderexceptionmodcrash",
            "multiple entries with same key",
            "failure message: ",
        ],
        advice: &[
            "日志里 “Caught exception from” 后面的名字就是出问题的 Mod，先把它禁用后重试。",
            "如果该 Mod 刚更新过，换回上一个版本；如果没有更新过，检查它是否支持当前游戏和加载器版本。",
            "禁用后能进游戏，再逐个开启其余 Mod 确认冲突对象。",
        ],
        links: &[],
        priority: 95,
        category: "mod",
    },
    Rule {
        id: "mod-init-failed",
        title: "Mod 初始化失败",
        summary: "某个 Mod 在构造或初始化阶段抛出异常，游戏还没进入主界面就退出了。",
        patterns: &[
            "failed to create mod instance",
            "exception during mod construction",
        ],
        advice: &[
            "按日志里的 ModID 找到对应 Mod，先禁用后重试。",
            "常见原因是 Mod 版本与游戏版本不匹配，或缺少它要求的前置。",
            "如果是整合包，优先检查最近手动加入的 Mod。",
        ],
        links: &[],
        priority: 94,
        category: "mod",
    },
    Rule {
        id: "fabric-entrypoint-crash",
        title: "Fabric Mod 入口点崩溃",
        summary: "Fabric 在执行某个 Mod 的入口点时出错，日志会写明 provided by 'xxx'。",
        patterns: &["could not execute entrypoint stage"],
        advice: &[
            "按日志里 provided by 后面的 Mod 名，把它更新到与当前 Minecraft 版本匹配的构建。",
            "如果更新后仍崩溃，先禁用该 Mod，确认游戏能正常启动。",
            "同时检查它的前置（Fabric API、Cloth Config 等）是否为同一版本线。",
        ],
        links: &[],
        priority: 94,
        category: "mod",
    },
    Rule {
        id: "fabric-solution",
        title: "加载器已给出解决方案",
        summary: "Fabric 在崩溃报告里直接写了 “A potential solution has been determined”，照着做通常就能解决。",
        patterns: &[
            "a potential solution has been determined",
            "potential solution has been determined",
        ],
        advice: &[
            "查看下方摘录中 “A potential solution has been determined” 之后的内容，按它给出的版本安装或替换 Mod。",
            "如果提示安装某个依赖，可在「下载 → 模组」里搜索并安装对应版本。",
            "安装完成后不要同时保留新旧两个版本。",
        ],
        links: &[],
        priority: 92,
        category: "mod",
    },
    Rule {
        id: "fabric-warnings",
        title: "Fabric 检测到不兼容的 Mod 组合",
        summary: "加载器列出了不兼容的 Mod 或版本范围，通常会在 Warnings / Incompatible mod set 段落中说明。",
        patterns: &[
            "warnings were found!",
            "incompatible mod set!",
            "incompatible mods found!",
            "mod resolution encountered",
        ],
        advice: &[
            "按日志中列出的 Mod 名称和版本要求，把其中一个换成兼容版本。",
            "优先更新报错里版本号较低或明显过旧的那个 Mod。",
            "如果提示两个 Mod 互斥，只保留其中一个。",
        ],
        links: &[],
        priority: 90,
        category: "mod",
    },
    Rule {
        id: "duplicate-mod-files",
        title: "同一个 Mod 装了多个版本",
        summary: "mods 目录里存在同一 Mod 的重复文件，加载器无法决定用哪一个。",
        patterns: &[
            "duplicatemodsfoundexception",
            "found a duplicate mod",
            "found duplicate mods",
            "duplicate mods found",
        ],
        advice: &[
            "在「版本设置 → 模组」里按名称排序，同一 Mod 只保留一个版本，通常保留较新的。",
            "注意区分同一 Mod 的不同加载器分支（Forge / Fabric / NeoForge），不要都放进去。",
            "删除前可以先移到桌面备份，确认能启动后再彻底删除。",
        ],
        links: &[],
        priority: 94,
        category: "mod",
    },
    Rule {
        id: "forge-java-incompatible",
        title: "Forge 与 Java 版本不兼容",
        summary: "旧版 Forge 使用了新版 Java 已移除的内部类，常见于 Java 8u321+ 搭配旧 Forge。",
        patterns: &["manifestentryverifier"],
        advice: &[
            "把 Forge 升级到 36.2.26 或更高（对应 1.16.5），或换回 Java 8u320 及更早的版本。",
            "不想动 Forge 时，在「设置 → Java」里选择一个 8u302 左右的 Java 8。",
            "升级 Forge 前先备份 versions 目录下的该实例。",
        ],
        links: &[],
        priority: 93,
        category: "java",
    },
    Rule {
        id: "optifine-world-load",
        title: "OptiFine 导致世界无法加载",
        summary: "进入世界时在区块加载阶段崩溃，堆栈指向 OptiFine 的 shouldForceTicks。",
        patterns: &["shouldforceticks", "chunkmanager$proxyticketmanager"],
        advice: &[
            "把 OptiFine 换成与当前 Forge 版本匹配的构建，或暂时移除 OptiFine 后进世界。",
            "如果只是为了光影，可改用 Iris + Sodium（Fabric）或对应版本的替代方案。",
            "换版本前先备份存档。",
        ],
        links: &[],
        priority: 92,
        category: "mod",
    },
    Rule {
        id: "mod-needs-java11",
        title: "Mod 需要 Java 11",
        summary: "某个 Mod 或库编译时使用了 Java 11 的类文件或 API，当前 Java 8 无法加载。",
        patterns: &[
            "java_11 could not be set",
            "unsafe.defineanonymousclass",
            "class file version 55.0",
            "class file version 55 ",
        ],
        advice: &[
            "在「设置 → Java」里选择一个 Java 11（或更高，按 Mod 要求）后重新启动。",
            "Forge 1.16.5 及以下通常用 Java 8，但部分 Mod 会要求 11；以日志中的版本号为准。",
            "切换 Java 后确认内存参数仍然合法。",
        ],
        links: &[],
        priority: 92,
        category: "java",
    },
    Rule {
        id: "java-32bit-exact",
        title: "使用的是 32 位 Java",
        summary: "32 位 Java 无法分配较大的堆内存，会以 Invalid maximum heap size 或无法保留空间的形式报错。",
        patterns: &[
            "invalid maximum heap size",
            "for 1048576kb object heap",
            "could not reserve enough space for object heap",
        ],
        advice: &[
            "安装 64 位 Java（Temurin 21 或 17，按游戏版本选择），然后在「设置 → Java」里选中它。",
            "确认系统本身是 64 位；32 位系统无法运行 64 位 Java。",
            "换 Java 后把最大内存调到 4096 MB 以上再试。",
        ],
        links: &[("下载 Temurin JDK", "https://adoptium.net/temurin/releases/")],
        priority: 93,
        category: "java",
    },
    Rule {
        id: "forge-incomplete-more",
        title: "Forge 安装不完整",
        summary: "Forge 的启动参数或模块文件缺失，常见于安装中断、文件被杀软删除或手动改过版本 json。",
        patterns: &[
            "invalid paths argument",
            "securemoduleclassloader",
            "could not find net/minecraft/client/minecraft.class",
        ],
        advice: &[
            "在版本设置里重装 Forge；「一键修复」会重新下载缺失的 libraries。",
            "检查 versions 目录下该实例的 json 是否被其他启动器改过。",
            "如果杀毒软件隔离了 Forge 文件，把游戏目录加入白名单后重装。",
        ],
        links: &[],
        priority: 92,
        category: "loader",
    },
    Rule {
        id: "repeat-installation",
        title: "加载器或 OptiFine 重复安装",
        summary: "启动参数里出现了两份加载器或两份 OptiFine，ModLauncher 会直接拒绝启动。",
        patterns: &[
            "multipleargumentsforoptionexception",
            "reads another module named",
        ],
        advice: &[
            "检查 versions 目录下该实例的 json，确认没有重复的加载器参数。",
            "OptiFine 只能保留一个 jar，不要同时放 Forge 版和独立版。",
            "无法判断时，重装一次加载器通常能清掉重复项。",
        ],
        links: &[],
        priority: 91,
        category: "loader",
    },
    Rule {
        id: "file-already-exists",
        title: "写文件时目标已存在",
        summary: "游戏或 Mod 尝试创建已存在的文件或目录，常见于上一次异常退出留下的残留。",
        patterns: &["filealreadyexistsexception", "the file already exists"],
        advice: &[
            "按日志里的路径找到该文件或文件夹，先重命名备份再启动。",
            "如果是 config 目录，可让游戏重新生成配置。",
            "不要直接删除整个游戏目录。",
        ],
        links: &[],
        priority: 88,
        category: "file",
    },
    Rule {
        id: "unsatisfied-link",
        title: "本地库缺失或位数不匹配",
        summary: "Java 找不到依赖的本地库（.dll/.so），通常是 libraries 不完整或 Java 位数不对。",
        patterns: &[
            "failed to locate library",
            "unsatisfiedlinkerror",
            "java.library.path",
        ],
        advice: &[
            "在版本设置里点「一键修复」，重新校验并下载 libraries。",
            "确认 Java 位数与系统一致（64 位系统用 64 位 Java）。",
            "如果只在某个 Mod 中出现，检查该 Mod 是否自带不兼容的本地库。",
        ],
        links: &[],
        priority: 88,
        category: "loader",
    },
    Rule {
        id: "sha1-digest-error",
        title: "文件校验失败",
        summary: "下载或解压后的文件与清单中的 SHA-1 不一致，说明文件损坏或被替换。",
        patterns: &["sha1 digest error"],
        advice: &[
            "删除日志中提到的文件，让启动器重新下载。",
            "检查磁盘是否有坏道或空间不足。",
            "如果反复校验失败，换一个下载源或稍后重试。",
        ],
        links: &[],
        priority: 88,
        category: "file",
    },
    Rule {
        id: "illegal-access",
        title: "Mod 访问了无权访问的类",
        summary: "某个 Mod 试图访问 Java 或游戏内部类，通常是 Mod 版本过旧或与其他 Mod 冲突。",
        patterns: &["illegalaccesserror", "tried to access class"],
        advice: &[
            "把堆栈中出现的 Mod 更新到支持当前游戏版本的构建。",
            "如果该 Mod 已停止更新，考虑换用替代品或移除。",
            "不要通过添加 JVM 参数强行开放访问，容易引发更多问题。",
        ],
        links: &[],
        priority: 86,
        category: "mod",
    },
    Rule {
        id: "module-resolution",
        title: "模块系统解析失败",
        summary: "Mod 的模块名非法、模块信息缺失或两个模块导出同一个包，常见于 jar 被改名或打包不规范。",
        patterns: &[
            "moduleresolution",
            "modulefind",
            "invalid module name",
            "unable to derive module descriptor",
        ],
        advice: &[
            "检查 mods 目录里是否有被手动改名的 jar，恢复原文件名。",
            "按日志中的模块名找到对应 Mod，更新到规范打包的版本。",
            "不要同时放同一个 Mod 的多个分支。",
        ],
        links: &[],
        priority: 85,
        category: "loader",
    },
    Rule {
        id: "java-agent-failed",
        title: "Java Agent 加载失败",
        summary: "启动参数里的 -javaagent 指向了不存在或损坏的 jar，JVM 在启动阶段就退出。",
        patterns: &[
            "processing of -javaagent failed",
            "error opening zip file or jar manifest missing",
        ],
        advice: &[
            "在版本设置的 Java 参数里删掉无效的 -javaagent 项。",
            "如果该项由某个 Mod 或皮肤补丁添加，重新安装该组件。",
            "清空自定义参数后仍失败时，检查启动脚本是否被手动改过。",
        ],
        links: &[],
        priority: 90,
        category: "java",
    },
    Rule {
        id: "fabric-loader-old",
        title: "Fabric Loader 版本过旧",
        summary: "旧版 Fabric Loader 缺少新版 Mixin 的类，启动时直接 NoClassDefFoundError。",
        patterns: &["fabricmixintransformerproxy"],
        advice: &[
            "在版本设置里把 Fabric Loader 升级到最新稳定版。",
            "升级后如果 Mod 不兼容，再逐个更新 Mod。",
            "不要混用不同加载器的 Mod jar。",
        ],
        links: &[],
        priority: 90,
        category: "loader",
    },
    Rule {
        id: "process-conflict",
        title: "游戏进程被占用或强制结束",
        summary: "文件被其他进程占用，或 Java 进程被 Windows 直接关闭，日志里会留下 WasClosedByWindows。",
        patterns: &[
            "wasclosedbywindows",
            "the process cannot access the file",
        ],
        advice: &[
            "在任务管理器里结束残留的 java.exe / javaw.exe 后重试。",
            "关闭同时运行的另一个启动器或游戏实例。",
            "把游戏目录加入杀毒软件白名单，避免扫描时锁文件。",
        ],
        links: &[],
        priority: 84,
        category: "file",
    },
    Rule {
        id: "curseforge-corrupted",
        title: "CurseForge 文件损坏",
        summary: "整合包或 Mod 的 CurseForge 元数据损坏，加载器无法读取。",
        patterns: &["curseforgecorrupted"],
        advice: &[
            "重新下载该整合包或 Mod 文件。",
            "不要手动编辑 CurseForge 的清单文件。",
            "如果是整合包，优先用启动器完整重装。",
        ],
        links: &[],
        priority: 84,
        category: "file",
    },
    Rule {
        id: "intel-driver",
        title: "Intel 显卡驱动崩溃",
        summary: "JVM 崩溃日志的 Problematic frame 指向 Intel 显卡驱动 DLL。",
        patterns: &["ig7icd64", "igdumd64", "ig9icd64", "ig10icd64", "ig12icd64"],
        advice: &[
            "到 Intel 官网或笔记本厂商页面更新核显驱动，不要只用 Windows 自动更新。",
            "如果更新后仍崩溃，尝试在显卡控制面板里为 java.exe 指定高性能显卡。",
            "临时可关闭光影和抗锯齿后再试。",
        ],
        links: &[("Intel 驱动下载", "https://www.intel.cn/content/www/cn/zh/download-center/home.html")],
        priority: 80,
        category: "graphics",
    },
    Rule {
        id: "amd-driver",
        title: "AMD 显卡驱动崩溃",
        summary: "JVM 崩溃日志的 Problematic frame 指向 AMD 显卡驱动 DLL（atio6axx / amdxx64 等）。",
        patterns: &["atio6axx", "atidxx64", "amdxx64", "amdxc64"],
        advice: &[
            "到 AMD 官网更新显卡驱动，安装时选择「清洁安装」。",
            "关闭 Radeon 的图像增强、Anti-Lag 等覆盖功能后重试。",
            "如果是笔记本双显卡，确认游戏使用的是独显。",
        ],
        links: &[("AMD 驱动下载", "https://www.amd.com/zh-cn/support/download/drivers.html")],
        priority: 80,
        category: "graphics",
    },
    Rule {
        id: "nvidia-driver",
        title: "NVIDIA 显卡驱动崩溃",
        summary: "JVM 崩溃日志的 Problematic frame 指向 NVIDIA OpenGL 驱动 DLL（nvoglv64 等）。",
        patterns: &["nvoglv64", "nvoglv32", "nvd3dum"],
        advice: &[
            "到 NVIDIA 官网更新显卡驱动，选择「清洁安装」。",
            "关闭 GeForce Experience 的游戏内覆盖后重试。",
            "笔记本用户确认 java.exe 使用的是独显而不是核显。",
        ],
        links: &[("NVIDIA 驱动下载", "https://www.nvidia.cn/Download/index.aspx")],
        priority: 80,
        category: "graphics",
    },
    Rule {
        id: "insufficient-memory-native",
        title: "系统内存不足（JVM 层）",
        summary: "JVM 无法继续申请本地内存，通常是物理内存或虚拟内存耗尽，而不是堆内存设置太小。",
        patterns: &["there is insufficient memory for the java runtime environment to continue"],
        advice: &[
            "关闭浏览器、视频和其他大型程序后重试。",
            "检查系统虚拟内存（页面文件）是否被禁用或设置得过小。",
            "如果物理内存本身较小，把游戏内存调低到 4096 MB 左右反而更稳定。",
        ],
        links: &[],
        priority: 86,
        category: "memory",
    },
    Rule {
        id: "optifine-performant",
        title: "OptiFine 与 Performant 冲突",
        summary: "Performant 的 Mixin 注入与 OptiFine 冲突，游戏在启动阶段崩溃。",
        patterns: &["onisonladder", "performant"],
        advice: &[
            "OptiFine 与 Performant 只能保留一个，通常保留 OptiFine。",
            "如果不用光影，可以移除 OptiFine 改用 Performant。",
            "不要通过改配置强行让两者共存。",
        ],
        links: &[],
        priority: 82,
        category: "mod",
    },
    Rule {
        id: "optifine-twilightforest",
        title: "OptiFine 与暮色森林冲突",
        summary: "暮色森林的渲染代码在 OptiFine 下出现 outside of image bounds。",
        patterns: &["outside of image bounds"],
        advice: &[
            "把 OptiFine 换成与当前 Forge 和暮色森林版本匹配的构建。",
            "如果仍冲突，进入暮色森林前临时移除 OptiFine。",
            "优先备份存档再调整 Mod。",
        ],
        links: &[],
        priority: 82,
        category: "mod",
    },
    Rule {
        id: "optifine-jade",
        title: "OptiFine 与 Jade 冲突",
        summary: "Jade 在 OptiFine 环境下访问 GameRenderer 的局部变量时崩溃。",
        patterns: &["m_109093_"],
        advice: &[
            "升级 Jade 和 OptiFine 到最新版本，两者都在持续修兼容问题。",
            "如果仍崩溃，暂时移除 Jade 或 OptiFine 其中一个。",
            "不要同时安装多个信息显示类 Mod（Jade / WAILA / HWM）。",
        ],
        links: &[],
        priority: 82,
        category: "mod",
    },
    Rule {
        id: "optifine-neoforge",
        title: "OptiFine 与 NeoForge 不兼容",
        summary: "NeoForge 在服务加载阶段发现 OptiFine 提供的非法实现，直接拒绝启动。",
        patterns: &["invalid services found"],
        advice: &[
            "NeoForge 不支持 OptiFine，移除 OptiFine 后启动。",
            "需要光影时改用 Iris / Oculus 等对应方案。",
            "不要试图用旧版 Forge 的 OptiFine jar 强行加载。",
        ],
        links: &[],
        priority: 86,
        category: "mod",
    },
    Rule {
        id: "optifine-forest",
        title: "OptiFine 与 Forest coremod 冲突",
        summary: "Forest 的 coremod 脚本与 OptiFine 的类转换冲突，启动阶段报 multipart.js 错误。",
        patterns: &["meta-inf/asm/multipart.js"],
        advice: &[
            "移除 OptiFine，或换用不含该 coremod 的 Forest 版本。",
            "确认 Forge 版本与该整合包要求一致。",
            "修改前备份 instances 目录。",
        ],
        links: &[],
        priority: 82,
        category: "mod",
    },
    Rule {
        id: "optifine-sodium",
        title: "OptiFine 与 Sodium 同时安装",
        summary: "两者都重写了渲染管线，同时安装会互相覆盖，几乎必然崩溃。",
        patterns: &["optifine"],
        advice: &[
            "只保留一个：要光影选 OptiFine，要性能选 Sodium（配 Iris）。",
            "不要通过禁用其中一个的配置文件来强行共存。",
            "移除后重启游戏，让渲染设置重新生成。",
        ],
        links: &[],
        priority: 88,
        category: "mod",
    },
    Rule {
        id: "shadersmod-optifine",
        title: "ShadersMod 与 OptiFine 冲突",
        summary: "旧版 ShadersMod 已经包含在 OptiFine 中，两者同时安装会重复转换渲染类。",
        patterns: &["shadersmod", "shaders mod"],
        advice: &[
            "删除 ShadersMod，光影功能直接用 OptiFine 自带的支持。",
            "如果是旧整合包，确认没有把 ShadersMod 又手动放回 mods。",
            "光影包放到 shaderpacks 目录，不要放进 mods。",
        ],
        links: &[],
        priority: 86,
        category: "mod",
    },
    Rule {
        id: "rtss-sodium",
        title: "RTSS 与 Sodium 不兼容",
        summary: "RivaTuner（RTSS）的覆盖层与 Sodium 的渲染线程冲突。",
        patterns: &["rtss"],
        advice: &[
            "关闭 RTSS 或把 java.exe 从 RTSS 的监控列表中移除。",
            "关闭 MSI Afterburner 的屏幕显示功能。",
            "如果不需帧数限制，直接退出 RTSS 后启动游戏。",
        ],
        links: &[],
        priority: 82,
        category: "mod",
    },
    Rule {
        id: "missing-embeddium",
        title: "Oculus 缺少 Embeddium",
        summary: "Oculus 需要 Embeddium（或 Sodium 的对应分支）作为前置，缺失时渲染初始化失败。",
        patterns: &["requires embeddium", "missing embeddium", "embeddium is required"],
        advice: &[
            "在「下载 → 模组」里安装与当前游戏版本匹配的 Embeddium。",
            "不要同时安装 Rubidium 和 Embeddium。",
            "装好后确认 Oculus 版本与 Embeddium 版本线一致。",
        ],
        links: &[],
        priority: 88,
        category: "mod",
    },
    Rule {
        id: "ctov-lithostitched",
        title: "CTOV 缺少 Lithostitched",
        summary: "ChoiceTheorem's Overhauled Village 需要 Lithostitched 作为前置。",
        patterns: &["ctov"],
        advice: &[
            "安装与当前游戏版本匹配的 Lithostitched。",
            "确认 CTOV 与 Lithostitched 的版本线一致。",
            "如果已安装仍报错，检查是否装成了 Fabric 分支。",
        ],
        links: &[],
        priority: 84,
        category: "mod",
    },
    Rule {
        id: "legacy-too-many-ids",
        title: "方块/物品 ID 超出上限",
        summary: "1.12.2 等旧版本的 ID 上限被 Mod 撑满，加载器无法分配新 ID。",
        patterns: &["legacytomanyids", "maximum id range exceeded"],
        advice: &[
            "安装 Just Enough IDs（JEID）后重试。",
            "或减少同时安装的内容类 Mod。",
            "JEID 只适用于 1.12.2 及更早版本，不要装到新版本。",
        ],
        links: &[(
            "Just Enough IDs（CurseForge）",
            "https://www.curseforge.com/minecraft/mc-mods/just-enough-ids",
        )],
        priority: 84,
        category: "mod",
    },
    Rule {
        id: "modernfix-watchdog",
        title: "ModernFix 看门狗触发",
        summary: "ModernFix 检测到启动或加载卡死并强制结束，日志里会出现 watchdog。",
        patterns: &["modernfix watchdog", "watchdog thread"],
        advice: &[
            "先把 ModernFix 更新到最新版本。",
            "如果只在大型整合包中出现，适当调大内存并减少同时加载的 Mod。",
            "仍无法解决时暂时移除 ModernFix 验证。",
        ],
        links: &[],
        priority: 82,
        category: "mod",
    },
    Rule {
        id: "connector-incompatible",
        title: "Sinytra Connector 不兼容",
        summary: "Connector 无法把某些 Fabric Mod 正确映射到 Forge/NeoForge，加载阶段直接报错。",
        patterns: &[
            "connectorincompatiblefabricmods",
            "sinytra connector",
        ],
        advice: &[
            "按日志里列出的 Mod，移除或换成 Forge 版本。",
            "把 Connector 和 Forgified Fabric API 更新到最新。",
            "Connector 不是万能的，渲染类 Fabric Mod 通常无法兼容。",
        ],
        links: &[],
        priority: 84,
        category: "mod",
    },
    Rule {
        id: "resource-location",
        title: "资源路径非法",
        summary: "某个 Mod 注册了包含大写字母或非法字符的资源 ID，1.21 起会直接拒绝。",
        patterns: &["resourcelocationexception"],
        advice: &[
            "按日志中的资源 ID 找到对应 Mod，更新到修复该问题的版本。",
            "不要手动修改 Mod jar 内的资源路径。",
            "如果是整合包，检查是否有旧版 Mod 混入。",
        ],
        links: &[],
        priority: 82,
        category: "mod",
    },
];

/// 没有命中任何具体规则、但明显是崩溃时的兜底条目。
fn generic_match() -> CrashMatch {
    CrashMatch {
        id: "unknown".into(),
        title: "游戏崩溃（未匹配到已知原因）".into(),
        summary: "已经生成崩溃报告，但没有命中启动器内置的常见问题规则。".into(),
        advice: vec![
            "查看下方摘录里的「Description」和第一段异常堆栈。".into(),
            "禁用最近新增的 Mod、资源包或光影后重试。".into(),
            "向社区求助时附上游戏版本、加载器版本和完整崩溃报告。".into(),
        ],
        evidence: Vec::new(),
        links: vec![CrashLink {
            label: "崩溃报告说明".into(),
            url: "https://minecraft.wiki/w/Crash_report".into(),
        }],
        priority: 10,
        phase: 4,
        confidence: "low".into(),
        category: "other".into(),
        suspects: Vec::new(),
    }
}

/// 日志几乎没有内容：通常是 Java 路径失效、被安全软件拦截，或 JVM 在初始化前就退出。
fn short_output_match() -> CrashMatch {
    CrashMatch {
        id: "short-output".into(),
        title: "启动输出过短，Java 没有真正运行".into(),
        summary: "游戏没有留下正常日志，Java 进程在初始化阶段就退出了。".into(),
        advice: vec![
            "在「设置 → Java」里重新选择 java.exe，确认路径存在且能正常运行。".into(),
            "检查杀毒软件是否拦截了 java.exe 或游戏目录。".into(),
            "在版本设置里清空自定义 Java 参数后重试。".into(),
        ],
        evidence: Vec::new(),
        links: Vec::new(),
        priority: 90,
        phase: 1,
        confidence: "medium".into(),
        category: "java".into(),
        suspects: Vec::new(),
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn modified_secs(path: &Path) -> u64 {
    fs::metadata(path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// 读日志：小文件整读，大文件保留「头部 + 尾部」，因为崩溃堆栈总在末尾。
fn read_text(path: &Path) -> String {
    let Ok(meta) = fs::metadata(path) else {
        return String::new();
    };
    let Ok(mut file) = File::open(path) else {
        return String::new();
    };
    let size = meta.len();
    if size <= WHOLE_LIMIT {
        let mut buffer = Vec::new();
        if file.read_to_end(&mut buffer).is_err() {
            return String::new();
        }
        return String::from_utf8_lossy(&buffer).to_string();
    }
    let mut head = vec![0u8; HEAD_LIMIT as usize];
    let head_len = file.read(&mut head).unwrap_or(0);
    let tail_start = size.saturating_sub(TAIL_LIMIT);
    let mut tail = Vec::new();
    if file.seek(SeekFrom::Start(tail_start)).is_ok() {
        let _ = file.read_to_end(&mut tail);
    }
    let mut text = String::from_utf8_lossy(&head[..head_len]).to_string();
    text.push_str(&format!(
        "\n\n... [日志过大，已省略中间 {} KB] ...\n\n",
        tail_start.saturating_sub(head_len as u64) / 1024
    ));
    text.push_str(&String::from_utf8_lossy(&tail));
    text
}

fn newest_file(dir: &Path, accept: impl Fn(&str) -> bool) -> Option<PathBuf> {
    let entries = fs::read_dir(dir).ok()?;
    let mut best: Option<(u64, PathBuf)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !accept(&name) {
            continue;
        }
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let modified = modified_secs(&path);
        if best
            .as_ref()
            .map(|(time, _)| modified > *time)
            .unwrap_or(true)
        {
            best = Some((modified, path));
        }
    }
    best.map(|(_, path)| path)
}

fn excerpt_from(text: &str, kind: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();
    let picked: Vec<String> = if kind == "latest-log" {
        lines
            .iter()
            .rev()
            .take(60)
            .rev()
            .map(|line| line.chars().take(240).collect())
            .collect()
    } else {
        lines
            .iter()
            .take(45)
            .map(|line| line.chars().take(240).collect())
            .collect()
    };
    picked
}

fn evidence_for(text: &str, patterns: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lowered = trimmed.to_lowercase();
        if !patterns.iter().any(|pattern| lowered.contains(pattern)) {
            continue;
        }
        // 带上行号，崩溃窗口里可以直接定位到日志原文
        let clipped: String = trimmed.chars().take(220).collect();
        let entry = format!("第 {} 行：{}", index + 1, clipped);
        if !out.contains(&entry) {
            out.push(entry);
        }
        if out.len() >= 4 {
            break;
        }
    }
    out
}

fn description_of(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let trimmed = line.trim();
        trimmed
            .strip_prefix("Description:")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

/// 规则分层：1 环境/安装 → 2 Mod 归因 → 3 兼容 → 4 兜底。
fn phase_of(id: &str) -> u8 {
    match id {
        "java-version"
        | "out-of-memory"
        | "vm-options"
        | "duplicate-classpath"
        | "loader-incomplete"
        | "corrupted-jar"
        | "file-locked"
        | "graphics"
        | "jvm-fatal"
        | "server-port"
        | "auth"
        | "authlib-login"
        | "java-32bit-exact"
        | "forge-java-incompatible"
        | "mod-needs-java11"
        | "forge-incomplete-more"
        | "repeat-installation"
        | "file-already-exists"
        | "unsatisfied-link"
        | "sha1-digest-error"
        | "java-agent-failed"
        | "fabric-loader-old"
        | "process-conflict"
        | "curseforge-corrupted"
        | "intel-driver"
        | "amd-driver"
        | "nvidia-driver"
        | "insufficient-memory-native" => 1,
        "customskinloader-version-detect"
        | "module-package-conflict"
        | "missing-dependency"
        | "mod-conflict"
        | "confirmed-mod-crash"
        | "mod-init-failed"
        | "fabric-entrypoint-crash"
        | "fabric-solution"
        | "fabric-warnings"
        | "duplicate-mod-files"
        | "manual-debug-crash" => 2,
        "mixin"
        | "mod-version-mismatch"
        | "class-loading"
        | "optifine-world-load"
        | "illegal-access"
        | "module-resolution"
        | "world-corrupt"
        | "optifine-performant"
        | "optifine-twilightforest"
        | "optifine-jade"
        | "optifine-neoforge"
        | "optifine-forest"
        | "optifine-sodium"
        | "shadersmod-optifine"
        | "rtss-sodium"
        | "oculus-embeddium"
        | "missing-indium"
        | "ferritecore-neighbor"
        | "geckolib-oculus"
        | "ctov-lithostitched"
        | "epicfight-addon"
        | "feature-order-cycle"
        | "modernfix-watchdog"
        | "language-provider-mismatch"
        | "legacy-too-many-ids"
        | "resource-location"
        | "server-config-corrupted"
        | "connector-incompatible"
        | "rubidium-deprecated"
        | "missing-embeddium" => 3,
        _ => 4,
    }
}

fn confidence_of(phase: u8) -> &'static str {
    match phase {
        1 | 2 => "high",
        3 => "medium",
        _ => "low",
    }
}

/// 组合规则需要多个关键词同时出现，避免单看一个通用词就误报。
fn rule_context_ok(id: &str, lowered: &str) -> bool {
    let required: &[&str] = match id {
        "optifine-world-load"
        | "optifine-performant"
        | "optifine-twilightforest"
        | "optifine-neoforge"
        | "optifine-forest" => &["optifine"],
        "optifine-jade" => &["optifine", "jade"],
        "optifine-sodium" => &["optifine", "sodium"],
        "shadersmod-optifine" => &["optifine"],
        "rtss-sodium" => &["rtss", "sodium"],
        "oculus-embeddium" => &["oculus"],
        "missing-indium" => &["indium"],
        "missing-embeddium" => &["embeddium"],
        "ferritecore-neighbor" => &["ferritecore"],
        "geckolib-oculus" => &["geckolib", "oculus"],
        "ctov-lithostitched" => &["ctov", "lithostitched"],
        "epicfight-addon" => &["epicfight"],
        "modernfix-watchdog" => &["modernfix"],
        "connector-incompatible" => &["connector"],
        "rubidium-deprecated" => &["rubidium"],
        // 认证失败必须确实用了第三方登录，避免把微软账号的报错也算到这里
        "authlib-login" => &["authlib"],
        _ => &[],
    };
    required.iter().all(|keyword| lowered.contains(keyword))
}

fn matches_in(text: &str) -> Vec<CrashMatch> {
    let lowered = text.to_lowercase();
    let mut result: Vec<CrashMatch> = Vec::new();
    for rule in RULES {
        if !rule.patterns.iter().any(|pattern| lowered.contains(pattern)) {
            continue;
        }
        if !rule_context_ok(rule.id, &lowered) {
            continue;
        }
        let phase = phase_of(rule.id);
        result.push(CrashMatch {
            id: rule.id.into(),
            title: rule.title.into(),
            summary: rule.summary.into(),
            advice: rule.advice.iter().map(|value| value.to_string()).collect(),
            evidence: evidence_for(text, rule.patterns),
            links: rule
                .links
                .iter()
                .map(|(label, url)| CrashLink {
                    label: label.to_string(),
                    url: url.to_string(),
                })
                .collect(),
            priority: rule.priority,
            phase,
            confidence: confidence_of(phase).into(),
            category: rule.category.into(),
            suspects: Vec::new(),
        });
    }
    // 远端规则：与内置规则同 id 时以内置为准，避免同一原因展示两遍
    for rule in remote_rules() {
        if RULES.iter().any(|builtin| builtin.id == rule.id) {
            continue;
        }
        if !rule_matches(&rule, &lowered) {
            continue;
        }
        if !rule_context_ok(&rule.id, &lowered) {
            continue;
        }
        let phase = rule.phase.unwrap_or_else(|| phase_of(&rule.id)).clamp(1, 4);
        let patterns: Vec<&str> = rule.patterns.iter().map(String::as_str).collect();
        result.push(CrashMatch {
            id: rule.id.clone(),
            title: rule.title.clone(),
            summary: if rule.summary.trim().is_empty() {
                rule.title.clone()
            } else {
                rule.summary.clone()
            },
            advice: rule.advice.clone(),
            evidence: evidence_for(text, &patterns),
            links: rule
                .links
                .iter()
                .map(|link| CrashLink {
                    label: link.label.clone(),
                    url: link.url.clone(),
                })
                .collect(),
            priority: rule.priority,
            phase,
            confidence: rule
                .confidence
                .clone()
                .unwrap_or_else(|| confidence_of(phase).to_string()),
            category: if rule.category.trim().is_empty() {
                "other".into()
            } else {
                rule.category.clone()
            },
            suspects: Vec::new(),
        });
    }
    if result.is_empty() {
        if looks_like_crash(text) {
            result.push(generic_match());
        }
        return result;
    }
    refine_matches(&mut result, &lowered);
    if result.is_empty() {
        return result;
    }
    // 只展示最早一层的命中：环境问题优先于 Mod 归因，再优先于兼容性和兜底
    let best_phase = result
        .iter()
        .map(|item| item.phase)
        .min()
        .unwrap_or(4);
    result.retain(|item| item.phase == best_phase);
    // 高优先级在前，同样优先级按规则表顺序
    result.sort_by(|a, b| b.priority.cmp(&a.priority));
    result
}

/// 命中多条规则时做互斥收敛，避免同一现象被重复归因。
fn refine_matches(matches: &mut Vec<CrashMatch>, lowered: &str) {
    let ids: Vec<String> = matches.iter().map(|item| item.id.clone()).collect();
    let has = |id: &str| ids.iter().any(|value| value == id);
    // 手动触发的调试崩溃没有修复价值，只保留这一条
    if has("manual-debug-crash") {
        matches.retain(|item| item.id == "manual-debug-crash");
        return;
    }
    // 已经定位到具体显卡驱动时，不再重复报笼统的 JVM/显卡问题
    if has("intel-driver") || has("amd-driver") || has("nvidia-driver") {
        matches.retain(|item| item.id != "jvm-fatal" && item.id != "graphics");
    }
    // 32 位 Java 报出的“内存不足 / 参数无效”都是假象，真正的修法是换 64 位 Java
    if has("java-32bit-exact") {
        matches.retain(|item| item.id != "vm-options");
        if !lowered.contains("java.lang.outofmemoryerror") && !lowered.contains("java heap space") {
            matches.retain(|item| item.id != "out-of-memory");
        }
    }
    // NightConfig 的报错只是配置损坏的表象，优先展示配置规则
    if has("mod-config") {
        matches.retain(|item| item.id != "nightconfig");
    }
    // OptiFine 世界加载失败已经点名，不再报笼统的存档损坏
    if has("optifine-world-load") {
        matches.retain(|item| item.id != "world-corrupt");
    }
}

fn looks_like_crash(text: &str) -> bool {
    let lowered = text.to_lowercase();
    lowered.contains("---- minecraft crash report ----")
        || lowered.contains("exception")
        || lowered.contains("caused by:")
        || lowered.contains("a fatal error has been detected")
}

fn problematic_frame(text: &str) -> Option<String> {
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        if line.trim().starts_with("# Problematic frame:") {
            if let Some(next) = lines.next() {
                let value = next.trim().trim_start_matches('#').trim();
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

/// 从 Java 模块冲突日志里提取可疑模块名，例如：
/// `Modules customskinloader and minecraft export package ...`
/// `Module fmlloader reads another module named fmlloader`
fn module_conflicts(text: &str) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        if let Some(start) = lower.find("modules ") {
            let tail = &line[start + 8..];
            let tail_lower = &lower[start + 8..];
            if let Some(export) = tail_lower.find(" export package ") {
                let names = &tail[..export];
                if let Some((left, right)) = names.split_once(" and ") {
                    push_module(left, &mut result);
                    push_module(right, &mut result);
                }
                // "export package ... to module radon"：radon 同时读取这两个模块，
                // 也列进可疑列表，方便用户在 mods 里定位到具体 jar
                if let Some(pos) = tail_lower[export..].find(" to module ") {
                    let reader = &tail[export + pos + 11..];
                    if let Some(name) = reader.split_whitespace().next() {
                        push_module(name, &mut result);
                    }
                }
            }
        }
        if let Some(start) = lower.find("reads another module named ") {
            let tail = &line[start + 27..];
            if let Some(name) = tail.split_whitespace().next() {
                push_module(name, &mut result);
            }
        }
    }
    result
}

fn push_module(name: &str, result: &mut Vec<String>) {
    let name = name
        .trim()
        .trim_matches(|ch: char| ch == '`' || ch == '\'' || ch == '"' || ch == ',' || ch == '.');
    if name.is_empty() || name.eq_ignore_ascii_case("minecraft") {
        return;
    }
    if !result.iter().any(|item| item.eq_ignore_ascii_case(name)) {
        result.push(name.to_string());
    }
}

/// 堆栈里出现频率最高的第三方包名（过滤掉游戏、加载器与常见库）。
fn suspicious_packages(text: &str) -> Vec<(String, usize)> {
    const IGNORED: &[&str] = &[
        "java.",
        "javax.",
        "jdk.",
        "sun.",
        "com.sun.",
        "net.minecraft.",
        "net.minecraftforge.",
        "net.neoforged.",
        "net.fabricmc.",
        "com.mojang.",
        "org.lwjgl.",
        "io.netty.",
        "oshi.",
        "com.google.",
        "org.apache.",
        "it.unimi.",
        "org.jetbrains.",
        "org.slf4j.",
        "ch.qos.logback.",
        "kotlin.",
        "scala.",
        "org.objectweb.asm.",
        "org.ow2.asm.",
        "cpw.mods.",
        "org.spongepowered.",
        "io.github.",
        "com.electronwill.",
        "org.commonmark.",
        "net.minecrell.",
        "org.jline.",
        "com.mojang.blaze3d.",
    ];
    let mut counts: HashMap<String, usize> = HashMap::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("at ") else {
            continue;
        };
        let Some(open) = rest.find('(') else {
            continue;
        };
        let full = &rest[..open];
        let segments: Vec<&str> = full.split('.').collect();
        // 至少 a.b.c.Class.method 才有包名可比
        if segments.len() < 5 {
            continue;
        }
        let package = segments[..segments.len() - 2].join(".");
        if package.split('.').count() < 2 {
            continue;
        }
        if IGNORED
            .iter()
            .any(|prefix| package.starts_with(prefix))
        {
            continue;
        }
        *counts.entry(package).or_insert(0) += 1;
    }
    let mut list: Vec<(String, usize)> = counts.into_iter().collect();
    list.sort_by(|a, b| b.1.cmp(&a.1));
    list.truncate(6);
    list
}

struct ModFile {
    file_name: String,
    mod_id: String,
    display_name: String,
    /// 斜杠形式的包前缀，例如 `com/example/mod`
    packages: Vec<String>,
}

fn toml_value(text: &str, key: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        let Some(value) = trimmed.strip_prefix(key) else {
            continue;
        };
        let value = value.trim_start();
        let Some(value) = value.strip_prefix('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').trim_matches('\'');
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

fn scan_mods(cfg: &Config, version_id: &str) -> Vec<ModFile> {
    let dir = instance::game_dir(cfg, version_id).join("mods");
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().to_string();
        let lower = file_name.to_lowercase();
        if !lower.ends_with(".jar") && !lower.ends_with(".jar.disabled") {
            continue;
        }
        let Ok(file) = File::open(&path) else {
            continue;
        };
        let Ok(mut zip) = zip::ZipArchive::new(file) else {
            continue;
        };
        let mut mod_id = String::new();
        let mut display_name = String::new();
        if let Ok(mut entry) = zip.by_name("fabric.mod.json") {
            let mut text = String::new();
            if entry.read_to_string(&mut text).is_ok() {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                    mod_id = json
                        .get("id")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        .to_string();
                    display_name = json
                        .get("name")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        .to_string();
                }
            }
        }
        if mod_id.is_empty() {
            for candidate in ["META-INF/mods.toml", "META-INF/neoforge.mods.toml"] {
                if let Ok(mut entry) = zip.by_name(candidate) {
                    let mut text = String::new();
                    if entry.read_to_string(&mut text).is_ok() {
                        mod_id = toml_value(&text, "modId").unwrap_or_default();
                        display_name = toml_value(&text, "displayName").unwrap_or_default();
                        if !mod_id.is_empty() {
                            break;
                        }
                    }
                }
            }
        }
        // 收集 jar 里的包前缀，用于和堆栈包名比对
        let mut packages: Vec<String> = Vec::new();
        for index in 0..zip.len().min(600) {
            let Ok(entry) = zip.by_index(index) else {
                continue;
            };
            let name = entry.name().to_string();
            if !name.ends_with(".class") || name.ends_with("module-info.class") {
                continue;
            }
            if let Some(position) = name.rfind('/') {
                let package = name[..position].to_string();
                if !package.is_empty() && !packages.contains(&package) {
                    packages.push(package);
                }
            }
            if packages.len() >= 200 {
                break;
            }
        }
        result.push(ModFile {
            file_name,
            mod_id,
            display_name,
            packages,
        });
    }
    result
}

fn locate_by_packages(mods: &[ModFile], packages: &[(String, usize)]) -> Vec<CrashSuspect> {
    let mut result: Vec<CrashSuspect> = Vec::new();
    for (package, _count) in packages {
        let normalized = package.replace('.', "/");
        let hit = mods.iter().find(|mod_file| {
            mod_file.packages.iter().any(|candidate| {
                candidate == &normalized
                    || candidate.starts_with(&format!("{normalized}/"))
                    || normalized.starts_with(&format!("{candidate}/"))
            })
        });
        let Some(hit) = hit else {
            continue;
        };
        if result
            .iter()
            .any(|item| item.file_name == hit.file_name)
        {
            continue;
        }
        result.push(CrashSuspect {
            name: if hit.display_name.is_empty() {
                if hit.mod_id.is_empty() {
                    hit.file_name.clone()
                } else {
                    hit.mod_id.clone()
                }
            } else {
                hit.display_name.clone()
            },
            file_name: hit.file_name.clone(),
            package: package.clone(),
            reason: "stack".into(),
        });
        if result.len() >= 3 {
            break;
        }
    }
    result
}

/// 从堆栈帧里提取可能对应 Mod 的关键字（类名 / 包名片段）。
/// 只在包名反查失败时兜底使用，所以过滤得比较狠，宁可漏报也不误报。
fn stack_keywords(text: &str) -> Vec<String> {
    const NOISE: &[&str] = &[
        "com", "org", "net", "io", "java", "javax", "jdk", "sun", "cpw", "mod", "mods", "api",
        "core", "util", "utils", "impl", "internal", "common", "client", "server", "main",
        "init", "event", "events", "config", "loader", "launch", "mixin", "mixins", "fabric",
        "forge", "neoforge", "minecraft", "mojang", "lwjgl", "spongepowered", "objectweb",
        "asm", "google", "apache", "jetbrains", "slf4j", "logback", "kotlin", "scala",
    ];
    let mut found: Vec<String> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("at ") else {
            continue;
        };
        let Some(open) = rest.find('(') else {
            continue;
        };
        let full = rest[..open].trim();
        // 去掉方法名只留类路径；Mixin 的 Foo$Bar 统一成点号
        let Some((class_path, _)) = full.rsplit_once('.') else {
            continue;
        };
        let normalized = class_path.replace('$', ".");
        for segment in normalized.split('.').take(4) {
            let word = segment.trim();
            if word.len() < 4 || word.chars().all(|ch| ch.is_ascii_digit()) {
                continue;
            }
            if NOISE.contains(&word.to_ascii_lowercase().as_str()) {
                continue;
            }
            if found.iter().any(|item| item.eq_ignore_ascii_case(word)) {
                continue;
            }
            found.push(word.to_string());
        }
        if found.len() >= 10 {
            break;
        }
    }
    found
}

/// 关键字与 Mod 的 id / 文件名做模糊比对，作为最后一步兜底。
fn locate_by_keywords(mods: &[ModFile], keywords: &[String]) -> Vec<CrashSuspect> {
    let mut result: Vec<CrashSuspect> = Vec::new();
    for keyword in keywords {
        let needle = keyword.to_ascii_lowercase().replace('_', "");
        if needle.len() < 4 {
            continue;
        }
        let hit = mods.iter().find(|mod_file| {
            let id = mod_file.mod_id.to_ascii_lowercase().replace('_', "");
            let file = mod_file.file_name.to_ascii_lowercase().replace('_', "");
            (id.len() >= 4 && (id.contains(&needle) || needle.contains(&id)))
                || file.contains(&needle)
        });
        let Some(hit) = hit else {
            continue;
        };
        if result.iter().any(|item| item.file_name == hit.file_name) {
            continue;
        }
        result.push(CrashSuspect {
            name: if hit.display_name.is_empty() {
                if hit.mod_id.is_empty() {
                    hit.file_name.clone()
                } else {
                    hit.mod_id.clone()
                }
            } else {
                hit.display_name.clone()
            },
            file_name: hit.file_name.clone(),
            package: keyword.clone(),
            reason: "stack".into(),
        });
        if result.len() >= 3 {
            break;
        }
    }
    result
}

/// 从崩溃报告的 "Suspected Mods:" 段与 "-- MOD xxx --" 标题提取 Mod。
fn reported_mod_tokens(text: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if trimmed.starts_with("Suspected Mods:") || trimmed.starts_with("Suspected Mod:") {
            if let Some((_, rest)) = trimmed.split_once(':') {
                let rest = rest.trim();
                if !rest.is_empty() {
                    tokens.push(rest.to_string());
                }
            }
            while let Some(next) = lines.peek() {
                let candidate = next.trim();
                let indented = next.starts_with('\t') || next.starts_with("    ") || next.starts_with("  ");
                if candidate.is_empty() || !indented {
                    break;
                }
                tokens.push(candidate.to_string());
                lines.next();
            }
        }
        if trimmed.starts_with("-- MOD ") && trimmed.ends_with(" --") {
            let name = trimmed
                .trim_start_matches("-- MOD ")
                .trim_end_matches(" --")
                .trim();
            if !name.is_empty() {
                tokens.push(name.to_string());
            }
        }
    }
    let mut normalized: Vec<String> = Vec::new();
    for token in tokens {
        let value = mod_token(&token);
        if value.is_empty() {
            continue;
        }
        if !normalized.iter().any(|item| item.eq_ignore_ascii_case(&value)) {
            normalized.push(value);
        }
    }
    normalized
}

/// "Better Mineshafts (bettermineshafts), Version: 1.0" → "bettermineshafts"
fn mod_token(entry: &str) -> String {
    if let (Some(open), Some(close)) = (entry.rfind('('), entry.rfind(')')) {
        if close > open {
            let inner = entry[open + 1..close].trim();
            if !inner.is_empty() {
                return inner.to_string();
            }
        }
    }
    entry
        .split(',')
        .next()
        .unwrap_or(entry)
        .trim()
        .to_string()
}

fn normalize_token(text: &str) -> String {
    text.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase())
        .collect()
}

fn locate_reported(mods: &[ModFile], tokens: &[String]) -> Vec<CrashSuspect> {
    let mut result: Vec<CrashSuspect> = Vec::new();
    for token in tokens {
        let normalized = normalize_token(token);
        if normalized.is_empty() {
            continue;
        }
        let hit = mods.iter().find(|mod_file| {
            let id = normalize_token(&mod_file.mod_id);
            let file = normalize_token(&mod_file.file_name);
            (!id.is_empty() && (id == normalized || normalized.contains(&id)))
                || file.contains(&normalized)
        });
        let suspect = match hit {
            Some(hit) => CrashSuspect {
                name: if hit.display_name.is_empty() {
                    token.clone()
                } else {
                    hit.display_name.clone()
                },
                file_name: hit.file_name.clone(),
                package: String::new(),
                reason: "reported".into(),
            },
            None => CrashSuspect {
                name: token.clone(),
                file_name: String::new(),
                package: String::new(),
                reason: "reported".into(),
            },
        };
        if result
            .iter()
            .any(|item| item.file_name == suspect.file_name && item.name == suspect.name)
        {
            continue;
        }
        result.push(suspect);
        if result.len() >= 3 {
            break;
        }
    }
    result
}

fn expected_java(game_version: &str) -> u32 {
    let mut parts = game_version.split('.');
    let major = parts
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(1);
    if major >= 21 {
        return 21;
    }
    if major != 1 {
        return 17;
    }
    let minor = parts
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(0);
    let patch = parts
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(0);
    if minor >= 21 || (minor == 20 && patch >= 5) {
        21
    } else if minor >= 17 {
        17
    } else {
        8
    }
}

fn build_context(cfg: &Config, version_id: &str) -> CrashContext {
    let (game_version, loader) = install_context(cfg, version_id);
    let required = crate::install::resolve_detail(cfg, version_id)
        .map(|merged| merged.detail.required_java())
        .unwrap_or(0);
    let java = crate::launch::find_java(cfg, required.max(8)).ok();
    let mod_count = count_mods(cfg, version_id);
    CrashContext {
        game_version,
        loader,
        java_major: java.as_ref().map(|info| info.major).unwrap_or(required),
        java_path: java
            .as_ref()
            .map(|info| info.path.clone())
            .unwrap_or_default(),
        memory_mb: crate::launch::effective_memory_mb(cfg),
        mod_count,
        problematic_frame: None,
    }
}

fn install_context(cfg: &Config, version_id: &str) -> (String, String) {
    crate::install::instance_context(cfg, version_id).unwrap_or_else(|_| (version_id.to_string(), String::new()))
}

fn count_mods(cfg: &Config, version_id: &str) -> u32 {
    let dir = instance::game_dir(cfg, version_id).join("mods");
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| {
                    let name = entry.file_name().to_string_lossy().to_lowercase();
                    name.ends_with(".jar") || name.ends_with(".jar.disabled")
                })
                .count() as u32
        })
        .unwrap_or(0)
}

/// 把实例信息写进建议，让提示更具体。
fn apply_context(matches: &mut [CrashMatch], context: &CrashContext) {
    if context.game_version.is_empty() && context.memory_mb == 0 {
        return;
    }
    let expected = if context.game_version.is_empty() {
        0
    } else {
        expected_java(&context.game_version)
    };
    for item in matches.iter_mut() {
        match item.id.as_str() {
            "java-version" => {
                if !context.game_version.is_empty() && expected > 0 {
                    let actual = if context.java_major == 0 {
                        "未检测到".to_string()
                    } else {
                        format!("Java {}", context.java_major)
                    };
                    item.advice.insert(
                        0,
                        format!(
                            "当前实例：{}，需要 Java {}；启动器目前选择的是 {}。",
                            context.game_version, expected, actual
                        ),
                    );
                    if context.java_major > 0 && context.java_major < expected {
                        item.advice.insert(
                            1,
                            "已确认 Java 版本偏低，请到「设置 → Java」安装并选中对应版本。".into(),
                        );
                    }
                }
            }
            "out-of-memory" | "vm-options" => {
                if context.memory_mb > 0 {
                    let hint = if context.memory_mb < 4096 {
                        "当前分配偏低，建议 4096–8192 MB"
                    } else {
                        "如果仍然崩溃，可再提高 1024–2048 MB（不超过物理内存一半）"
                    };
                    item.advice.insert(
                        0,
                        format!("当前最大内存 {} MB；{}。", context.memory_mb, hint),
                    );
                }
            }
            "mod-version-mismatch" | "mod-conflict" | "mixin" | "missing-dependency" => {
                if !context.game_version.is_empty() || !context.loader.is_empty() {
                    item.advice.insert(
                        0,
                        format!(
                            "当前实例：{} · {}，已安装 {} 个 Mod。",
                            if context.game_version.is_empty() {
                                "未知版本"
                            } else {
                                &context.game_version
                            },
                            if context.loader.is_empty() {
                                "加载器未知"
                            } else {
                                &context.loader
                            },
                            context.mod_count
                        ),
                    );
                }
            }
            "jvm-fatal" => {
                if let Some(frame) = &context.problematic_frame {
                    item.advice.insert(
                        0,
                        format!("本次崩溃的 Problematic frame：{frame}，可据此判断是显卡驱动还是本地库。"),
                    );
                }
            }
            _ => {}
        }
    }
}

fn attach_suspects(matches: &mut [CrashMatch], suspects: &[CrashSuspect]) {
    if suspects.is_empty() {
        return;
    }
    if let Some(first) = matches.iter_mut().find(|item| item.category == "mod") {
        first.suspects = suspects.iter().take(3).cloned().collect();
    }
}

/// 启动器日志里是否出现 JVM 启动阶段的致命错误
fn launcher_fatal(text: &str) -> bool {
    let lowered = text.to_lowercase();
    [
        "exception in thread",
        "duplicate key",
        "unsupportedclassversionerror",
        "could not find or load main class",
        "error occurred during initialization of vm",
        "a fatal error has been detected",
        "invalid maximum heap size",
        "unrecognized option",
    ]
    .iter()
    .any(|pattern| lowered.contains(pattern))
}

/// 成功启动后记录 mods 快照，崩溃时用它对比出“这次改了什么”。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SnapshotEntry {
    file_name: String,
    size: u64,
    modified: u64,
    /// 文件内容摘要；旧版快照没有这个字段，留空时退回按大小/时间判断
    #[serde(default)]
    sha256: String,
}

fn safe_version_id(version_id: &str) -> String {
    version_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

/// 游戏目录短哈希：区分不同游戏文件夹下的同名版本，避免快照互相覆盖。
fn short_path_hash(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    let text = path.to_string_lossy().to_lowercase();
    format!("{:x}", Sha256::digest(text.as_bytes()))
        .chars()
        .take(8)
        .collect()
}

fn snapshot_path(version_id: &str, mods_dir: &Path) -> PathBuf {
    // 实例目录 = mods 的上级目录；不同游戏目录下同名版本必须分开存
    let instance = mods_dir.parent().unwrap_or(mods_dir);
    let key = format!(
        "{}-{}",
        safe_version_id(version_id),
        short_path_hash(instance)
    );
    crate::config::data_dir()
        .join("mod-snapshots")
        .join(format!("{key}.json"))
}

fn scan_mod_entries(mods_dir: &Path) -> Vec<SnapshotEntry> {
    let mut entries = Vec::new();
    let Ok(dir) = fs::read_dir(mods_dir) else {
        return entries;
    };
    for entry in dir.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|value| value.to_str()) != Some("jar") {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        entries.push(SnapshotEntry {
            file_name: entry.file_name().to_string_lossy().to_string(),
            size: meta.len(),
            modified: meta
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs())
                .unwrap_or(0),
            sha256: sha256_of(&path).unwrap_or_default(),
        });
    }
    entries.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    entries
}

fn sha256_of(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    let file = File::open(path).ok()?;
    let mut reader = std::io::BufReader::new(file);
    let mut hasher = Sha256::new();
    std::io::copy(&mut reader, &mut hasher).ok()?;
    Some(format!("{:x}", hasher.finalize()))
}

/// 同一文件名下内容是否变化；旧快照没有摘要时退回大小与修改时间。
fn snapshot_changed(old: &SnapshotEntry, now: &SnapshotEntry) -> bool {
    if !old.sha256.is_empty() && !now.sha256.is_empty() {
        return old.sha256 != now.sha256;
    }
    old.size != now.size || old.modified != now.modified
}

/// 成功退出（代码 0）后调用，覆盖上一份快照。
pub fn save_mod_snapshot(version_id: &str, mods_dir: &Path) {
    let entries = scan_mod_entries(mods_dir);
    let path = snapshot_path(version_id, mods_dir);
    let Some(parent) = path.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(bytes) = serde_json::to_vec_pretty(&entries) else {
        return;
    };
    let tmp = path.with_extension("json.tmp");
    if fs::write(&tmp, bytes).is_ok() {
        let _ = fs::rename(&tmp, &path);
    }
}

/// 对比上一份成功快照与当前 mods，返回新增/删除/修改的文件。
fn diff_mod_snapshot(version_id: &str, mods_dir: &Path) -> Vec<ModChange> {
    let Ok(text) = fs::read_to_string(snapshot_path(version_id, mods_dir)) else {
        return Vec::new();
    };
    let Ok(previous) = serde_json::from_str::<Vec<SnapshotEntry>>(&text) else {
        return Vec::new();
    };
    if previous.is_empty() {
        return Vec::new();
    }
    let current = scan_mod_entries(mods_dir);
    let mut old: HashMap<String, SnapshotEntry> = HashMap::new();
    for entry in previous {
        old.insert(entry.file_name.clone(), entry);
    }
    let mut changes = Vec::new();
    for entry in &current {
        match old.remove(&entry.file_name) {
            None => changes.push(ModChange {
                kind: "added".into(),
                file_name: entry.file_name.clone(),
            }),
            Some(previous) if snapshot_changed(&previous, entry) => changes.push(ModChange {
                kind: "modified".into(),
                file_name: entry.file_name.clone(),
            }),
            Some(_) => {}
        }
    }
    for file_name in old.keys() {
        changes.push(ModChange {
            kind: "removed".into(),
            file_name: file_name.clone(),
        });
    }
    changes.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    changes
}

/// 撤销快照之后新增的 Mod：移到 mods-disabled 保留现场，返回被移走的文件名。
pub fn undo_added_mods(version_id: &str, game_dir: &Path) -> Result<Vec<String>> {
    let mods_dir = game_dir.join("mods");
    let added: Vec<String> = diff_mod_snapshot(version_id, &mods_dir)
        .into_iter()
        .filter(|change| change.kind == "added")
        .map(|change| change.file_name)
        .collect();
    if added.is_empty() {
        return Ok(added);
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0);
    let target = game_dir
        .join("mods-disabled")
        .join(format!("undo-{stamp}"));
    fs::create_dir_all(&target)?;
    let mut moved = Vec::new();
    for name in added {
        let source = mods_dir.join(&name);
        if !source.is_file() {
            continue;
        }
        if fs::rename(&source, target.join(&name)).is_ok() {
            moved.push(name);
        }
    }
    if moved.is_empty() {
        bail!("新增的 Mod 已不在 mods 目录，或没有移动权限");
    }
    Ok(moved)
}

/// 读取崩溃前后 5 分钟的 Windows 应用日志。失败或超时都返回空，不影响分析。
#[cfg(windows)]
fn windows_crash_events(anchor: u64) -> Vec<WindowsCrashEvent> {
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;

    if anchor == 0 {
        return Vec::new();
    }
    let start = anchor.saturating_sub(300);
    let end = anchor.saturating_add(300);
    let command = format!(
        "$start=[DateTimeOffset]::FromUnixTimeSeconds({start}).UtcDateTime; $end=[DateTimeOffset]::FromUnixTimeSeconds({end}).UtcDateTime; Get-WinEvent -FilterHashtable @{{LogName='Application'; Id=1000,1001,1002,1005; StartTime=$start; EndTime=$end}} -MaxEvents 20 | ForEach-Object {{ [PSCustomObject]@{{ EventId=$_.Id; Provider=$_.ProviderName; TimeCreated=$_.TimeCreated.ToUniversalTime().ToString('o'); Message=$_.Message }} }} | ConvertTo-Json -Compress"
    );
    let Ok(mut child) = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &command])
        .creation_flags(0x0800_0000)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return Vec::new();
    };
    // 管道缓冲只有约 4KB：必须边等边读，否则输出一多 PowerShell 就会写满阻塞，
    // 4 秒后被当成卡死杀掉，结果永远为空
    let reader = child.stdout.take().map(|mut out| {
        std::thread::spawn(move || {
            let mut buffer = Vec::new();
            let _ = out.read_to_end(&mut buffer);
            buffer
        })
    });
    // PowerShell 偶尔会卡住，最多等 4 秒
    let mut finished = false;
    for _ in 0..40 {
        if matches!(child.try_wait(), Ok(Some(_))) {
            finished = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    if !finished {
        let _ = child.kill();
    }
    let status_ok = child.wait().map(|status| status.success()).unwrap_or(false);
    let stdout = reader
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    if !status_ok {
        return Vec::new();
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&stdout) else {
        return Vec::new();
    };
    let entries = match value {
        serde_json::Value::Array(items) => items,
        serde_json::Value::Object(_) => vec![value],
        _ => return Vec::new(),
    };
    entries
        .into_iter()
        .filter_map(|entry| {
            Some(WindowsCrashEvent {
                event_id: entry.get("EventId")?.as_u64()? as u32,
                provider: entry.get("Provider")?.as_str().unwrap_or_default().to_string(),
                time_created: entry.get("TimeCreated")?.as_str()?.to_string(),
                message: entry
                    .get("Message")?
                    .as_str()
                    .unwrap_or_default()
                    .chars()
                    .take(600)
                    .collect(),
            })
        })
        .collect()
}

#[cfg(not(windows))]
fn windows_crash_events(_anchor: u64) -> Vec<WindowsCrashEvent> {
    Vec::new()
}

pub fn analyze(cfg: &Config, version_id: &str, exit_code: Option<i32>) -> Result<CrashReport> {
    let game_dir = instance::game_dir(cfg, version_id);
    // 与上一次成功启动的 mods 快照对比，直接圈出这次改动了哪些 Mod
    let mod_changes = diff_mod_snapshot(version_id, &game_dir.join("mods"));
    let latest_log = game_dir.join("logs").join("latest.log");
    // latest.log 会被截断，debug.log 更完整；两者都读，堆栈更不容易丢
    let debug_log = game_dir.join("logs").join("debug.log");
    // 启动器的 stderr 记录：JVM 在 Minecraft 日志初始化前失败（如类路径重复、
    // Java 参数错误）时，错误只出现在这里，游戏目录里的 latest.log 是上一次运行
    let launcher_log = crate::config::data_dir().join("logs").join("game.log");
    let launcher_text = read_text(&launcher_log);
    let launcher_for_version = launcher_text
        .lines()
        .next()
        .map(|line| line.contains(version_id))
        .unwrap_or(false);
    // 只认启动阶段的致命错误；正常运行时 stderr 也可能有一些普通告警
    let launcher_failed =
        launcher_for_version && launcher_text.contains("[stderr]") && launcher_fatal(&launcher_text);
    let crash_report = newest_file(&game_dir.join("crash-reports"), |name| {
        name.starts_with("crash-") && name.ends_with(".txt")
    });
    let hs_err = newest_file(&game_dir, |name| {
        name.starts_with("hs_err_pid") && name.ends_with(".log")
    });

    let latest_modified = modified_secs(&latest_log);
    let crash_fresh = crash_report
        .as_ref()
        .map(|path| modified_secs(path) + SAME_RUN_WINDOW >= latest_modified)
        .unwrap_or(false);
    let hs_err_fresh = hs_err
        .as_ref()
        .map(|path| modified_secs(path) + SAME_RUN_WINDOW >= latest_modified)
        .unwrap_or(false);

    let primary = if crash_fresh {
        crash_report.clone()
    } else if hs_err_fresh {
        hs_err.clone()
    } else if launcher_failed {
        // 比旧的 latest.log 更能反映这次启动失败的原因
        Some(launcher_log.clone())
    } else if latest_log.exists() {
        Some(latest_log.clone())
    } else {
        None
    };

    let mut scanned = Vec::new();
    for (path, label, exists) in [
        (crash_report.as_ref(), "crash-report", crash_report.is_some()),
        (hs_err.as_ref(), "hs-err", hs_err.is_some()),
        (Some(&latest_log), "latest-log", latest_log.exists()),
        (Some(&debug_log), "debug-log", debug_log.exists()),
        (Some(&launcher_log), "launcher-log", launcher_failed),
    ] {
        if let Some(path) = path {
            if exists && path.exists() {
                scanned.push(format!("{} · {}", label, path.display()));
            }
        }
    }

    let mut context = build_context(cfg, version_id);
    let Some(primary) = primary else {
        return Ok(CrashReport {
            version_id: version_id.into(),
            found: false,
            source: String::new(),
            source_path: String::new(),
            source_kind: "none".into(),
            modified: 0,
            headline: "还没有可分析的日志".into(),
            description: None,
            matches: Vec::new(),
            excerpt: Vec::new(),
            scanned,
            suspects: Vec::new(),
            mod_changes: Vec::new(),
            windows_events: Vec::new(),
            context,
        });
    };

    let source_kind = if Some(&primary) == crash_report.as_ref() {
        "crash-report"
    } else if Some(&primary) == hs_err.as_ref() {
        "hs-err"
    } else if launcher_failed && primary == launcher_log {
        "launcher-log"
    } else {
        "latest-log"
    };

    let text = read_text(&primary);
    let mut combined = text.clone();
    if latest_log.exists() && latest_log != primary {
        combined.push('\n');
        combined.push_str(&read_text(&latest_log));
    }
    if launcher_failed && launcher_log != primary {
        combined.push('\n');
        combined.push_str(&launcher_text);
    }
    if let Some(path) = hs_err.as_ref() {
        if *path != primary {
            combined.push('\n');
            combined.push_str(&read_text(path));
        }
    }
    if debug_log.exists() && debug_log != primary {
        combined.push('\n');
        combined.push_str(&read_text(&debug_log));
    }

    context.problematic_frame = problematic_frame(&combined);

    let description = description_of(&text);
    // 退出码闸门：正常退出（0）且没有新的崩溃报告时，
    // 日志里的异常只是历史残留，不应判成崩溃
    let clean_exit = exit_code == Some(0) && !crash_fresh && !hs_err_fresh && !launcher_failed;
    // 崩溃后读一下 Windows 应用日志：识别“进程被系统结束”“应用未响应”等没有 Java 堆栈的情况
    let windows_events = if clean_exit {
        Vec::new()
    } else {
        windows_crash_events(modified_secs(&primary))
    };
    let mut matches = if clean_exit {
        Vec::new()
    } else {
        matches_in(&combined)
    };
    // 日志几乎没有内容时，给一个比“未匹配到已知原因”更具体的结论
    if matches.is_empty() && !clean_exit && text.trim().chars().count() < 100 {
        matches.push(short_output_match());
    }

    // 可疑 Mod：堆栈包名优先，其次是模块冲突名，最后是崩溃报告自带的 Suspected Mods
    let packages = suspicious_packages(&combined);
    let modules = module_conflicts(&combined);
    // 正常退出时不需要扫 mods（读几百个 jar 很慢），也不该给出可疑 Mod
    let mods = if clean_exit
        || (packages.is_empty() && modules.is_empty() && !combined.contains("Suspected Mod"))
    {
        Vec::new()
    } else {
        scan_mods(cfg, version_id)
    };
    let mut suspects = locate_by_packages(&mods, &packages);
    if suspects.len() < 3 {
        for suspect in locate_reported(&mods, &modules) {
            if suspects
                .iter()
                .any(|item| item.file_name == suspect.file_name && item.name == suspect.name)
            {
                continue;
            }
            suspects.push(suspect);
            if suspects.len() >= 3 {
                break;
            }
        }
    }
    if suspects.len() < 3 {
        let reported = reported_mod_tokens(&combined);
        for suspect in locate_reported(&mods, &reported) {
            if suspects
                .iter()
                .any(|item| item.file_name == suspect.file_name && item.name == suspect.name)
            {
                continue;
            }
            suspects.push(suspect);
            if suspects.len() >= 3 {
                break;
            }
        }
    }
    // 包名对不上任何 jar 时，用堆栈里的类名 / 包名片段再做一次模糊匹配
    if suspects.len() < 3 && !mods.is_empty() {
        let keywords = stack_keywords(&combined);
        for suspect in locate_by_keywords(&mods, &keywords) {
            if suspects.iter().any(|item| item.file_name == suspect.file_name) {
                continue;
            }
            suspects.push(suspect);
            if suspects.len() >= 3 {
                break;
            }
        }
    }

    // CustomSkinLoader 的模块冲突非常常见，命中时直接给出针对该 Mod 的结论
    let csl_conflict = modules
        .iter()
        .any(|name| normalize_token(name).contains("customskinloader"));
    if csl_conflict {
        if let Some(item) = matches
            .iter_mut()
            .find(|item| item.id == "module-package-conflict")
        {
            item.summary =
                "CustomSkinLoader 的 ForgeV1 / Legacy 版本与 1.17 及以上版本的模块系统冲突。".into();
            item.advice.insert(
                0,
                "删除 mods 里 CustomSkinLoader 的 ForgeV1 / Legacy jar，以及实例目录下的 CustomSkinLoader 文件夹；1.18.2 请改用 ForgeV2 / ForgeActive 版本，或直接不装（DOCL 已内置第三方皮肤同步）。"
                    .into(),
            );
        }
    }

    apply_context(&mut matches, &context);
    attach_suspects(&mut matches, &suspects);

    let headline = if clean_exit {
        "上次运行正常退出（代码 0），日志里的异常不代表崩溃".into()
    } else if source_kind == "crash-report" {
        description
            .as_ref()
            .map(|value| format!("崩溃原因：{value}"))
            .unwrap_or_else(|| "检测到崩溃报告".into())
    } else if source_kind == "hs-err" {
        "检测到 Java 虚拟机崩溃日志".into()
    } else if source_kind == "launcher-log" {
        "启动阶段失败：Java 进程没有正常启动".into()
    } else if matches.is_empty() {
        "日志里没有发现明显错误".into()
    } else {
        "日志里发现可疑错误".into()
    };

    Ok(CrashReport {
        version_id: version_id.into(),
        found: true,
        source: primary
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_default(),
        source_path: primary.to_string_lossy().to_string(),
        source_kind: source_kind.into(),
        modified: modified_secs(&primary),
        headline,
        description,
        matches,
        excerpt: excerpt_from(&text, source_kind),
        scanned,
        suspects,
        mod_changes,
        windows_events,
        context,
    })
}

/* ---------- 导出报告 ---------- */

/// 含账号凭据的日志行整行隐藏，避免导出后泄露。
fn redact(text: &str) -> (String, usize) {
    const KEYS: [&str; 5] = [
        "accesstoken",
        "refreshtoken",
        "clienttoken",
        "authorization: bearer",
        "bearer ",
    ];
    let mut hidden = 0usize;
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let lower = line.to_lowercase();
        if KEYS.iter().any(|key| lower.contains(key)) {
            hidden += 1;
            out.push_str("[已隐藏：包含账号凭据的日志行]\n");
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    (out, hidden)
}

fn analysis_text(report: &CrashReport) -> String {
    let mut out = String::new();
    out.push_str("DOCL 崩溃分析报告\n");
    out.push_str("=================\n\n");
    out.push_str(&format!("版本：{}\n", report.version_id));
    out.push_str(&format!("游戏版本：{}\n", report.context.game_version));
    out.push_str(&format!("加载器：{}\n", report.context.loader));
    out.push_str(&format!(
        "Java：{}（{}）\n",
        report.context.java_major, report.context.java_path
    ));
    out.push_str(&format!("最大内存：{} MB\n", report.context.memory_mb));
    out.push_str(&format!("已安装 Mod：{} 个\n", report.context.mod_count));
    if let Some(frame) = &report.context.problematic_frame {
        out.push_str(&format!("Problematic frame：{frame}\n"));
    }
    out.push_str(&format!("\n结论：{}\n", report.headline));
    if let Some(description) = &report.description {
        out.push_str(&format!("崩溃描述：{description}\n"));
    }
    if !report.scanned.is_empty() {
        out.push_str("\n扫描的日志：\n");
        for item in &report.scanned {
            out.push_str(&format!("  - {item}\n"));
        }
    }
    out.push_str("\n可疑 Mod：\n");
    if report.suspects.is_empty() {
        out.push_str("  （未定位到具体 Mod）\n");
    } else {
        for item in &report.suspects {
            out.push_str(&format!(
                "  - {}（{}）{}{}\n",
                item.name,
                if item.file_name.is_empty() {
                    "未匹配到文件"
                } else {
                    &item.file_name
                },
                if item.package.is_empty() {
                    String::new()
                } else {
                    format!("包名 {}", item.package)
                },
                if item.reason == "stack" { " · 来自堆栈" } else { " · 崩溃报告标注" }
            ));
        }
    }
    out.push_str("\n匹配结果：\n");
    for item in &report.matches {
        out.push_str(&format!(
            "\n[{}] {}（优先级 {}）\n{}\n",
            item.id, item.title, item.priority, item.summary
        ));
        for tip in &item.advice {
            out.push_str(&format!("  · {tip}\n"));
        }
        if !item.evidence.is_empty() {
            out.push_str("  证据：\n");
            for line in &item.evidence {
                out.push_str(&format!("    {line}\n"));
            }
        }
    }
    out
}

pub fn export(cfg: &Config, version_id: &str) -> Result<String> {
    export_to(cfg, version_id, &crate::config::data_dir().join("crash-exports"))
}

/// 导出误报反馈：用户标记为「不是这个原因」的规则 + 实例上下文，便于提交反馈。
pub fn export_feedback(cfg: &Config, version_id: &str, rule_ids: &[String]) -> Result<String> {
    let report = analyze(cfg, version_id, None)?;
    let rules: Vec<serde_json::Value> = report
        .matches
        .iter()
        .filter(|item| rule_ids.is_empty() || rule_ids.iter().any(|id| id == &item.id))
        .map(|item| {
            // 证据行来自原始日志，反馈会被用户转发，先隐藏含凭据的行
            let evidence: Vec<String> = item
                .evidence
                .iter()
                .map(|line| redact(line).0.trim_end().to_string())
                .collect();
            serde_json::json!({
                "id": item.id,
                "title": item.title,
                "summary": item.summary,
                "category": item.category,
                "priority": item.priority,
                "evidence": evidence,
            })
        })
        .collect();
    let payload = serde_json::json!({
        "type": "false-positive-feedback",
        "created": now_secs(),
        "launcher_version": crate::appupdate::CURRENT_VERSION,
        "os": std::env::consts::OS,
        "version_id": version_id,
        "source_kind": report.source_kind,
        "game_version": report.context.game_version,
        "loader": report.context.loader,
        "java_major": report.context.java_major,
        "mod_count": report.context.mod_count,
        "rules": rules,
    });
    let dir = crate::config::data_dir().join("crash-feedback");
    fs::create_dir_all(&dir)?;
    let safe_id: String = version_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let path = dir.join(format!("DOCL-feedback-{safe_id}-{}.json", now_secs()));
    fs::write(&path, serde_json::to_vec_pretty(&payload)?)?;
    Ok(path.to_string_lossy().to_string())
}

/// 用户主动分享时，把脱敏后的分析结果与相关日志上传到 mclo.gs，返回链接。
pub async fn share(
    client: &reqwest::Client,
    cfg: &Config,
    version_id: &str,
    report: &CrashReport,
) -> Result<String> {
    let game_dir = instance::game_dir(cfg, version_id);
    let mut text = analysis_text(report);
    let mut paths: Vec<PathBuf> = Vec::new();
    if !report.source_path.is_empty() {
        paths.push(PathBuf::from(&report.source_path));
    }
    paths.push(game_dir.join("logs").join("latest.log"));
    if let Some(path) = newest_file(&game_dir, |name| {
        name.starts_with("hs_err_pid") && name.ends_with(".log")
    }) {
        paths.push(path);
    }
    for path in paths {
        if !path.is_file() {
            continue;
        }
        text.push_str(&format!("\n\n===== {} =====\n", path.display()));
        text.push_str(&read_text(&path));
    }
    // 上传到公开站点前，把用户目录替换掉
    if let Ok(home) = std::env::var("USERPROFILE") {
        if !home.trim().is_empty() {
            text = text.replace(&home, "%USERPROFILE%");
        }
    }
    let (redacted, _) = redact(&text);
    if redacted.len() > 9 * 1024 * 1024 {
        bail!("日志超过 9 MB，mclo.gs 无法接收，请改用「导出报告」");
    }
    let response = client
        .post("https://api.mclo.gs/1/log")
        .form(&[("content", redacted)])
        .send()
        .await?;
    let value: serde_json::Value = response.json().await?;
    if value.get("success").and_then(|item| item.as_bool()) != Some(true) {
        let reason = value
            .get("error")
            .and_then(|item| item.as_str())
            .unwrap_or("mclo.gs 返回失败");
        bail!("上传日志失败：{reason}");
    }
    value
        .get("url")
        .and_then(|item| item.as_str())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("mclo.gs 没有返回链接"))
}

fn export_to(cfg: &Config, version_id: &str, dir: &Path) -> Result<String> {
    // 导出是用户主动操作，拿不到本次退出码，按正常流程分析
    let report = analyze(cfg, version_id, None)?;
    fs::create_dir_all(dir)?;
    let safe_id: String = version_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let path = dir.join(format!("DOCL-crash-{safe_id}-{}.zip", now_secs()));
    let file = File::create(&path)?;
    let mut writer = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    let mut hidden_total = 0usize;

    fn add_text(
        writer: &mut zip::ZipWriter<File>,
        name: &str,
        content: &str,
        hidden: &mut usize,
    ) -> Result<()> {
        let (redacted, lines) = redact(content);
        *hidden += lines;
        writer.start_file(name, zip::write::SimpleFileOptions::default())?;
        writer.write_all(redacted.as_bytes())?;
        Ok(())
    }

    add_text(&mut writer, "analysis.txt", &analysis_text(&report), &mut hidden_total)?;

    let game_dir = instance::game_dir(cfg, version_id);
    let mut index = 1usize;
    let mut log_paths: Vec<PathBuf> = Vec::new();
    if !report.source_path.is_empty() {
        log_paths.push(PathBuf::from(&report.source_path));
    }
    log_paths.push(game_dir.join("logs").join("latest.log"));
    if let Some(path) = newest_file(&game_dir, |name| {
        name.starts_with("hs_err_pid") && name.ends_with(".log")
    }) {
        log_paths.push(path);
    }
    // 启动器 stderr 记录：启动阶段失败（JVM 没起来）时只有这里有错误
    let launcher_log = crate::config::data_dir().join("logs").join("game.log");
    let belongs_to_version = read_text(&launcher_log)
        .lines()
        .next()
        .map(|line| line.contains(version_id))
        .unwrap_or(false);
    if belongs_to_version && !log_paths.iter().any(|path| path == &launcher_log) {
        log_paths.push(launcher_log);
    }
    for path in log_paths {
        if !path.is_file() {
            continue;
        }
        let text = read_text(&path);
        let name = format!(
            "logs/{index}-{}",
            path.file_name()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_else(|| "log.txt".into())
        );
        add_text(&mut writer, &name, &text, &mut hidden_total)?;
        index += 1;
    }

    let context_json = serde_json::to_string_pretty(&report.context)?;
    add_text(&mut writer, "context.json", &context_json, &mut hidden_total)?;

    // 附上当前 Mod 清单和与上次成功启动的差异，方便社区/作者定位
    let mods_json = serde_json::to_string_pretty(&scan_mod_entries(&game_dir.join("mods")))?;
    add_text(&mut writer, "mods.json", &mods_json, &mut hidden_total)?;
    let changes_json = serde_json::to_string_pretty(&report.mod_changes)?;
    add_text(
        &mut writer,
        "mod_changes.json",
        &changes_json,
        &mut hidden_total,
    )?;

    // 记录隐藏了多少行，便于用户判断报告是否被裁剪
    writer.start_file("README.txt", options)?;
    writer.write_all(
        format!(
            "由 DOCL 导出的崩溃报告\n\n包含：分析结论、可疑 Mod、实例信息与相关日志。\n出于安全考虑，有 {hidden_total} 行包含账号凭据的日志已被隐藏。\n"
        )
        .as_bytes(),
    )?;
    writer.finish()?;
    Ok(path.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_java_version_mismatch() {
        let text = "java.lang.UnsupportedClassVersionError: net/fabricmc/loader/api/FabricLoader has been compiled by a more recent version of the Java Runtime (class file version 65.0)";
        let matches = matches_in(text);
        assert_eq!(matches[0].id, "java-version");
        assert!(!matches[0].evidence.is_empty());
        assert_eq!(matches[0].priority, 90);
    }

    /// CSL 读不到 version.json 时会用旧补丁直接崩，不能再被误判成 VC++ 缺失
    #[test]
    fn detects_customskinloader_version_detection_failure() {
        let text = "java.lang.IllegalStateException: Transformation rule 'customskinloader:skin-manager-patch/skin-manager.constructor.v1' selected net/minecraft/client/resources/SkinManager for protocol=0, world=0 but did not modify any bytecode. If you want to force CustomSkinLoader to ignore this patch failure, add -Dcustomskinloader.ignorePatchFailure=true to the JVM arguments.";
        let report = matches_in(text);
        assert_eq!(report[0].id, "customskinloader-version-detect");
        assert!(report[0].priority >= 96);
    }

    /// 只加载了 authlib-injector 的普通崩溃，不能判成第三方认证失败
    #[test]
    fn authlib_injector_alone_is_not_auth_failure() {
        let text = "JVM Flags: 5 total; -XX:HeapDumpPath=MojangTricksIntelDriversForPerformance\n\
-javaagent:C:\\Users\\x\\authlib-injector.jar=https://littleskin.cn/api/yggdrasil\n\
[authlib-injector] [INFO] Version: 1.2.5\n\
[authlib-injector] [INFO] Authentication server: https://littleskin.cn/api/yggdrasil\n\
---- Minecraft Crash Report ----\njava.lang.NullPointerException: Cannot invoke ...";
        let matches = matches_in(text);
        assert!(!matches.iter().any(|item| item.id == "authlib-login"));
    }

    /// 真正的认证错误仍然要命中，并且保留高优先级
    #[test]
    fn authlib_failure_still_detected() {
        let text = "[authlib-injector] [INFO] Version: 1.2.5\n\
com.mojang.authlib.exceptions.AuthenticationException: Failed to authenticate";
        let matches = matches_in(text);
        let hit = matches.iter().find(|item| item.id == "authlib-login");
        assert!(hit.is_some());
        assert_eq!(hit.unwrap().phase, 1);
    }

    #[test]
    fn detects_memory_and_graphic_issue() {
        let text = "java.lang.OutOfMemoryError: Java heap space\nGLFW error 65543: Failed to create window";
        let report = matches_in(text);
        let ids: Vec<&str> = report.iter().map(|item| item.id.as_str()).collect();
        assert!(ids.contains(&"out-of-memory"));
        assert_eq!(report[0].id, "out-of-memory");
    }

    /// 分层短路：低一层的原因不应该和高优先层一起堆到结果里
    #[test]
    fn short_circuits_on_earliest_phase() {
        let text = "java.lang.OutOfMemoryError: Java heap space\nGLFW error 65543: Failed to create window";
        let report = matches_in(text);
        assert!(!report.is_empty());
        assert!(report
            .iter()
            .all(|item| item.phase == report[0].phase));
    }

    /// 包名对不上 jar 时，用堆栈里的类名 / 包名片段兜底定位 Mod
    #[test]
    fn stack_keywords_locate_mod_by_name() {
        let text = "\tat com.example.coolmod.mixin.CoreMixin$Inner.apply(CoreMixin.java:12)\n\tat net.minecraft.client.Minecraft.run(Minecraft.java:1)";
        let keywords = stack_keywords(text);
        assert!(keywords
            .iter()
            .any(|item| item.eq_ignore_ascii_case("coolmod")));
        assert!(!keywords
            .iter()
            .any(|item| item.eq_ignore_ascii_case("minecraft")));

        let mods = vec![ModFile {
            file_name: "CoolMod-1.0.jar".into(),
            mod_id: "coolmod".into(),
            display_name: "Cool Mod".into(),
            packages: Vec::new(),
        }];
        let suspects = locate_by_keywords(&mods, &keywords);
        assert_eq!(suspects.len(), 1);
        assert_eq!(suspects[0].file_name, "CoolMod-1.0.jar");
    }

    /// 退出码为 0 时，latest.log 里的历史异常不应该被当成崩溃
    #[test]
    fn clean_exit_suppresses_stale_log_errors() {
        let stamp = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!("docl-crash-exit-{stamp}"));
        let logs = root.join("versions").join("testpack").join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(
            logs.join("latest.log"),
            "java.lang.OutOfMemoryError: Java heap space\n",
        )
        .unwrap();

        let mut cfg = crate::config::Config::default();
        cfg.game_dir = root.to_string_lossy().to_string();
        cfg.version_isolation = true;

        let clean = analyze(&cfg, "testpack", Some(0)).unwrap();
        assert!(clean.found);
        assert!(clean.matches.is_empty(), "正常退出不应产生崩溃结论");

        let crashed = analyze(&cfg, "testpack", Some(1)).unwrap();
        assert!(crashed
            .matches
            .iter()
            .any(|item| item.id == "out-of-memory"));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 外部启动器生成的清单重复列库时，NeoForge 引导器直接报 Duplicate key
    #[test]
    fn detects_duplicate_classpath_libraries() {
        let text = "Exception in thread \"main\" java.lang.IllegalStateException: Duplicate key D:\\libraries\\com\\google\\code\\gson\\gson\\2.10.1\\gson-2.10.1.jar\n\tat cpw.mods.niofs.union.UnionFileSystem.<init>(UnionFileSystem.java:134)";
        let matches = matches_in(text);
        assert_eq!(matches[0].id, "duplicate-classpath");
        assert_eq!(matches[0].priority, 92);
    }

    /// 加载器安装不完整时，Java 找不到引导类
    #[test]
    fn detects_incomplete_loader_install() {
        let text = "Error: Could not find or load main class cpw.mods.bootstraplauncher\nCaused by: java.lang.ClassNotFoundException: cpw.mods.bootstraplauncher";
        let matches = matches_in(text);
        assert_eq!(matches[0].id, "loader-incomplete");
        assert_eq!(matches[0].priority, 90);
    }

    /// 游戏没关干净时文件被另一个进程占用
    #[test]
    fn detects_locked_files() {
        let text = "java.nio.file.FileSystemException: D:\\instances\\1.20.1-forge\\options.txt: The process cannot access the file because it is being used by another process.";
        let matches = matches_in(text);
        assert!(matches.iter().any(|item| item.id == "file-locked"));
        assert!(matches.iter().all(|item| item.id != "missing-dependency"));
    }

    /// 1.18.2 里 CustomSkinLoader 的 ForgeV1 分支会和原版模块争抢同一个包
    #[test]
    fn detects_module_package_conflict() {
        let text = "java.lang.module.ResolutionException: Modules customskinloader and minecraft export package net.minecraft.client.renderer to module radon\n\tat cpw.mods.modlauncher.ModuleLayerHandler.buildLayer(ModuleLayerHandler.java:80)";
        let matches = matches_in(text);
        assert_eq!(matches[0].id, "module-package-conflict");
        assert_eq!(matches[0].priority, 90);
    }

    #[test]
    fn extracts_conflicting_module_names() {
        let text = "java.lang.module.ResolutionException: Modules customskinloader and minecraft export package net.minecraft.client.renderer to module radon\nModule radon reads another module named customskinloader";
        let modules = module_conflicts(text);
        assert_eq!(modules.len(), 2);
        assert!(modules.iter().any(|name| name == "customskinloader"));
        assert!(modules.iter().any(|name| name == "radon"));
        assert!(!modules.iter().any(|name| name.eq_ignore_ascii_case("minecraft")));
    }

    /// 模块名要能对上 mods 目录里的 jar，才能定位到具体文件
    #[test]
    fn maps_module_names_to_installed_mods() {
        let mods = vec![
            ModFile {
                file_name: "CustomSkinLoader_ForgeV1-14.17.2.jar".into(),
                mod_id: "customskinloader".into(),
                display_name: "CustomSkinLoader".into(),
                packages: Vec::new(),
            },
            ModFile {
                file_name: "radon-1.0.jar".into(),
                mod_id: "radon".into(),
                display_name: "Radon".into(),
                packages: Vec::new(),
            },
        ];
        let suspects = locate_reported(&mods, &["customskinloader".into(), "radon".into()]);
        assert_eq!(suspects.len(), 2);
        assert_eq!(suspects[0].file_name, "CustomSkinLoader_ForgeV1-14.17.2.jar");
        assert_eq!(suspects[0].reason, "reported");
        assert_eq!(suspects[1].file_name, "radon-1.0.jar");
    }

    /// Mixin 找不到可选目标的 ClassNotFoundException 是正常告警，不能报成缺前置
    #[test]
    fn mixin_optional_class_warnings_are_not_dependency_errors() {
        let text = "[main/WARN] [mixin/]: Error loading class: xaero/common/patreon/Patreon (java.lang.ClassNotFoundException: xaero.common.patreon.Patreon)";
        let matches = matches_in(text);
        assert!(matches.iter().all(|item| item.id != "missing-dependency"));
    }

    #[test]
    fn recognizes_fatal_launcher_output() {
        assert!(launcher_fatal(
            "[stderr] Exception in thread \"main\" java.lang.IllegalStateException: Duplicate key"
        ));
        assert!(!launcher_fatal(
            "[stderr] WARN Could not find method in optional class"
        ));
    }

    #[test]
    fn falls_back_to_generic_crash() {
        let text = "---- Minecraft Crash Report ----\nDescription: Rendering overlay\nsomething odd";
        let matches = matches_in(text);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].id, "unknown");
    }

    #[test]
    fn extracts_suspicious_packages() {
        let text = "\tat com.example.coolmod.Core.tick(Core.java:10)\n\tat net.minecraft.client.Minecraft.run(Minecraft.java:1)\n\tat com.example.coolmod.Core.tick(Core.java:20)";
        let packages = suspicious_packages(text);
        assert_eq!(packages[0].0, "com.example.coolmod");
        assert_eq!(packages[0].1, 2);
    }

    #[test]
    fn reads_reported_suspected_mods() {
        let text = "Suspected Mods: \n\tExample Mod (examplemod), Version: 1.0\n\tOther (othermod), Version: 2.0\n\n-- MOD thirdmod --\n";
        let tokens = reported_mod_tokens(text);
        assert_eq!(tokens, vec!["examplemod", "othermod", "thirdmod"]);
    }

    #[test]
    fn expected_java_by_game_version() {
        assert_eq!(expected_java("1.12.2"), 8);
        assert_eq!(expected_java("1.16.5"), 8);
        assert_eq!(expected_java("1.17.1"), 17);
        assert_eq!(expected_java("1.20.4"), 17);
        assert_eq!(expected_java("1.20.5"), 21);
        assert_eq!(expected_java("1.21.1"), 21);
    }

    #[test]
    fn redacts_credential_lines() {
        let (text, hidden) = redact("normal line\n--accessToken abcdef\nkey=value");
        assert_eq!(hidden, 1);
        assert!(text.contains("normal line"));
        assert!(!text.contains("abcdef"));
    }

    #[test]
    fn reads_newest_crash_report_from_game_dir() {
        let stamp = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("docl-crash-test-{stamp}"));
        let reports = dir.join("crash-reports");
        std::fs::create_dir_all(&reports).unwrap();
        std::fs::write(
            reports.join("crash-2026-01-01_00.00.00-client.txt"),
            "---- Minecraft Crash Report ----\nDescription: Test",
        )
        .unwrap();
        let found = newest_file(&reports, |name| {
            name.starts_with("crash-") && name.ends_with(".txt")
        });
        assert!(found.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 端到端：构造一个带 Mod 的实例，验证堆栈定位、报告标注与导出 ZIP。
    #[test]
    fn locates_mod_from_stack_and_exports() {
        let stamp = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!("docl-crash-e2e-{stamp}"));
        let instance_dir = root.join("versions").join("testpack");
        let mods_dir = instance_dir.join("mods");
        std::fs::create_dir_all(&mods_dir).unwrap();

        // 造一个含 com/example/coolmod 包的 Mod
        {
            let file = File::create(mods_dir.join("coolmod-1.0.jar")).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("fabric.mod.json", options).unwrap();
            zip.write_all(br#"{"id":"coolmod","name":"Cool Mod"}"#)
                .unwrap();
            zip.start_file("com/example/coolmod/Core.class", options)
                .unwrap();
            zip.write_all(b"\xca\xfe\xba\xbe").unwrap();
            zip.finish().unwrap();
        }

        let reports = instance_dir.join("crash-reports");
        std::fs::create_dir_all(&reports).unwrap();
        let crash_text = "---- Minecraft Crash Report ----\nDescription: Ticking entity\n\nCaused by: java.lang.NoClassDefFoundError: com/example/coolmod/Missing\n\tat com.example.coolmod.Core.tick(Core.java:10)\n\tat net.minecraft.client.Minecraft.run(Minecraft.java:1)\n\nSuspected Mods: \n\tCool Mod (coolmod), Version: 1.0\n";
        std::fs::write(
            reports.join("crash-2026-01-01_00.00.00-client.txt"),
            crash_text,
        )
        .unwrap();
        let logs = instance_dir.join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(logs.join("latest.log"), "old run\n").unwrap();

        let mut cfg = crate::config::Config::default();
        cfg.game_dir = root.to_string_lossy().to_string();
        cfg.version_isolation = true;

        let report = analyze(&cfg, "testpack", None).unwrap();
        assert!(report.found);
        assert_eq!(report.source_kind, "crash-report");
        assert!(
            report
                .suspects
                .iter()
                .any(|item| item.file_name == "coolmod-1.0.jar"),
            "未从堆栈定位到 Mod：{:?}",
            report.suspects
        );
        assert!(report
            .matches
            .iter()
            .any(|item| item.category == "mod" && !item.suspects.is_empty()));

        let out = root.join("exports");
        std::fs::create_dir_all(&out).unwrap();
        let path = export_to(&cfg, "testpack", &out).unwrap();
        assert!(Path::new(&path).is_file());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 仓库根目录的规则文件会由 CDN 直接下发，格式写错就等于线上规则不可用。
    #[test]
    fn shipped_rules_file_is_valid() {
        let file: RemoteRuleFile = serde_json::from_str(include_str!("../../crash-rules.json"))
            .expect("规则文件应当可解析");
        let rules = sanitize(file.rules);
        assert!(!rules.is_empty());
        assert!(rules.iter().all(|rule| !rule.patterns.is_empty()));
        // id 重复会让两条规则互相覆盖，直接拒绝
        let mut ids: Vec<&str> = rules.iter().map(|rule| rule.id.as_str()).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "规则 id 存在重复");
    }

    #[test]
    fn remote_rules_are_sanitized() {
        let rules = sanitize(vec![
            RemoteRule {
                id: "test-remote".into(),
                title: "远端测试规则".into(),
                summary: String::new(),
                patterns: vec!["REMOTE_TOKEN".into()],
                advice: vec!["按提示处理".into()],
                links: Vec::new(),
                priority: 64,
                category: "mod".into(),
                match_mode: String::new(),
                phase: None,
                confidence: None,
            },
            RemoteRule {
                id: "   ".into(),
                title: "缺少 id".into(),
                summary: String::new(),
                patterns: vec!["x".into()],
                advice: Vec::new(),
                links: Vec::new(),
                priority: 1,
                category: String::new(),
                match_mode: "all".into(),
                phase: None,
                confidence: None,
            },
        ]);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].patterns[0], "remote_token");
        // 缺省或写错一律按 any 处理
        assert_eq!(rules[0].match_mode, "any");
        assert!(rule_matches(&rules[0], "java.lang.exception: remote_token found"));
    }

    #[test]
    fn rule_match_all_requires_every_pattern() {
        let rule = RemoteRule {
            id: "test-all".into(),
            title: "组合规则".into(),
            summary: String::new(),
            patterns: vec!["alpha".into(), "beta".into()],
            advice: Vec::new(),
            links: Vec::new(),
            priority: 70,
            category: "mod".into(),
            match_mode: "all".into(),
            phase: None,
            confidence: None,
        };
        assert!(rule_matches(&rule, "alpha and beta"));
        assert!(!rule_matches(&rule, "alpha only"));

        let any_rule = RemoteRule {
            match_mode: "any".into(),
            ..rule.clone()
        };
        assert!(rule_matches(&any_rule, "alpha only"));
    }

    #[test]
    fn local_rule_ids_are_unique() {
        let mut ids: Vec<&str> = RULES.iter().map(|rule| rule.id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "内置规则 id 存在重复");
    }

    #[test]
    fn detects_confirmed_mod_crash_with_evidence_line() {
        let text = "java.lang.RuntimeException: boom\n\tCaught exception from coolmod (coolmod)\n\tat coolmod.Core.onLoad(Core.java:1)";
        let report = matches_in(text);
        let item = report
            .iter()
            .find(|item| item.id == "confirmed-mod-crash")
            .expect("应命中确定 Mod 崩溃");
        assert_eq!(item.phase, 2);
        assert_eq!(item.confidence, "high");
        assert!(item
            .evidence
            .iter()
            .any(|line| line.contains("第 2 行") && line.contains("Caught exception")));
    }

    #[test]
    fn java_32bit_suppresses_memory_and_vm_rules() {
        let text = "Error occurred during initialization of VM\nInvalid maximum heap size: -Xmx4096M\nCould not reserve enough space for 1048576KB object heap";
        let report = matches_in(text);
        let ids: Vec<&str> = report.iter().map(|item| item.id.as_str()).collect();
        assert!(ids.contains(&"java-32bit-exact"));
        assert!(!ids.contains(&"out-of-memory"));
        assert!(!ids.contains(&"vm-options"));
    }

    #[test]
    fn optifine_world_rule_requires_optifine_and_wins_over_world_corrupt() {
        let without = matches_in("ChunkManager$ProxyTicketManager.shouldForceTicks boom");
        assert!(without
            .iter()
            .all(|item| item.id != "optifine-world-load"));

        let with = matches_in(
            "OptiFine\nChunkManager$ProxyTicketManager.shouldForceTicks\nFailed to load world",
        );
        let ids: Vec<&str> = with.iter().map(|item| item.id.as_str()).collect();
        assert!(ids.contains(&"optifine-world-load"));
        assert!(!ids.contains(&"world-corrupt"));
    }

    #[test]
    fn manual_debug_crash_short_circuits_other_rules() {
        let report = matches_in("Manually triggered debug crash\njava.lang.OutOfMemoryError");
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].id, "manual-debug-crash");
    }

    #[test]
    fn detects_duplicate_mod_installation() {
        let report = matches_in("java.lang.RuntimeException: Found a duplicate mod coolmod at /mods/coolmod-1.0.jar");
        assert!(report
            .iter()
            .any(|item| item.id == "duplicate-mod-files"));
    }

    /// 同名文件内容不同时必须算改动，不能只看大小和时间
    #[test]
    fn snapshot_uses_sha256_when_available() {
        let base = SnapshotEntry {
            file_name: "coolmod-1.0.jar".into(),
            size: 100,
            modified: 1000,
            sha256: "aaa".into(),
        };
        let same = SnapshotEntry {
            sha256: "aaa".into(),
            ..base.clone()
        };
        let replaced = SnapshotEntry {
            sha256: "bbb".into(),
            ..base.clone()
        };
        assert!(!snapshot_changed(&base, &same));
        assert!(snapshot_changed(&base, &replaced));
    }

    /// 旧快照没有摘要，退回按大小/修改时间判断
    #[test]
    fn snapshot_falls_back_without_sha256() {
        let old = SnapshotEntry {
            file_name: "coolmod-1.0.jar".into(),
            size: 100,
            modified: 1000,
            sha256: String::new(),
        };
        let same = SnapshotEntry {
            sha256: String::new(),
            ..old.clone()
        };
        let touched = SnapshotEntry {
            modified: 2000,
            sha256: String::new(),
            ..old.clone()
        };
        assert!(!snapshot_changed(&old, &same));
        assert!(snapshot_changed(&old, &touched));
    }

    /// 规则回滚要把上一版换回来，并且可以再点一次回到刚才的版本
    #[test]
    fn rollback_swaps_rule_files() {
        let stamp = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("docl-rules-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        let cache = dir.join("crash-rules.json");
        let prev = dir.join("crash-rules.prev.json");
        let old = r#"{"rules":[{"id":"old-rule","title":"旧规则","patterns":["boom"]}]}"#;
        let new = r#"{"rules":[{"id":"new-rule","title":"新规则","patterns":["bang"]}]}"#;
        std::fs::write(&cache, new).unwrap();
        std::fs::write(&prev, old).unwrap();

        let (rules, bytes) = swap_rule_files(&cache, &prev).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].id, "old-rule");
        assert_eq!(bytes, old.as_bytes());
        assert_eq!(std::fs::read_to_string(&cache).unwrap(), old);
        assert_eq!(std::fs::read_to_string(&prev).unwrap(), new);

        // 再回滚一次回到新规则
        let (rules, _) = swap_rule_files(&cache, &prev).unwrap();
        assert_eq!(rules[0].id, "new-rule");

        // 没有上一版时明确报错
        std::fs::remove_file(&prev).unwrap();
        assert!(swap_rule_files(&cache, &prev).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
