use crate::config::Config;
use crate::instance;
use anyhow::Result;
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
    /// java / memory / mod / loader / graphics / file / world / server / auth
    pub category: String,
    pub suspects: Vec<CrashSuspect>,
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

/// 启动时先读本地缓存，保证离线时也能用上一次拉到的规则。
pub fn load_cached_rules() {
    let Ok(text) = fs::read_to_string(rules_cache_path()) else {
        return;
    };
    let Ok(file) = serde_json::from_str::<RemoteRuleFile>(&text) else {
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
    let count = rules.len();
    if let Ok(mut guard) = remote_store().write() {
        *guard = rules;
    }
    if fs::create_dir_all(crate::config::data_dir()).is_ok() {
        let _ = fs::write(rules_cache_path(), &bytes);
    }
    Ok(count)
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
            "在「设置 → Java」里安装或选择匹配的 Java：1.20.5+ 需要 Java 21，1.17+ 需要 17，1.16 及以下用 8。",
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
        patterns: &[
            "authlib-injector",
            "yggdrasil",
            "failed to authenticate",
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
        category: "other".into(),
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
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lowered = trimmed.to_lowercase();
        if !patterns.iter().any(|pattern| lowered.contains(pattern)) {
            continue;
        }
        let clipped: String = trimmed.chars().take(240).collect();
        if !out.contains(&clipped) {
            out.push(clipped);
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

fn matches_in(text: &str) -> Vec<CrashMatch> {
    let lowered = text.to_lowercase();
    let mut result: Vec<CrashMatch> = Vec::new();
    for rule in RULES {
        if !rule.patterns.iter().any(|pattern| lowered.contains(pattern)) {
            continue;
        }
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
    // 分级短路：只保留命中的最高一级，避免次要规则把主因淹没
    let best_tier = result
        .iter()
        .map(|item| tier_of(item.priority))
        .min()
        .unwrap_or(0);
    result.retain(|item| tier_of(item.priority) == best_tier);
    // 高优先级在前，同样优先级按规则表顺序
    result.sort_by(|a, b| b.priority.cmp(&a.priority));
    result
}

/// 规则分层：0 = 高优先（几乎可以断定原因），1 = 中等，2 = 兜底提示。
fn tier_of(priority: u8) -> u8 {
    if priority >= 86 {
        0
    } else if priority >= 70 {
        1
    } else {
        2
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

pub fn analyze(cfg: &Config, version_id: &str, exit_code: Option<i32>) -> Result<CrashReport> {
    let game_dir = instance::game_dir(cfg, version_id);
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
    let mut matches = if clean_exit {
        Vec::new()
    } else {
        matches_in(&combined)
    };

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

    #[test]
    fn detects_memory_and_graphic_issue() {
        let text = "java.lang.OutOfMemoryError: Java heap space\nGLFW error 65543: Failed to create window";
        let report = matches_in(text);
        let ids: Vec<&str> = report.iter().map(|item| item.id.as_str()).collect();
        assert!(ids.contains(&"out-of-memory"));
        assert_eq!(report[0].id, "out-of-memory");
    }

    /// 分级短路：低一级的规则不应该和高优先规则一起堆到结果里
    #[test]
    fn short_circuits_on_highest_tier() {
        let text = "java.lang.OutOfMemoryError: Java heap space\nGLFW error 65543: Failed to create window";
        let report = matches_in(text);
        assert!(report.iter().all(|item| item.id != "graphics"));
        assert!(report
            .iter()
            .all(|item| tier_of(item.priority) == tier_of(report[0].priority)));
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
        };
        assert!(rule_matches(&rule, "alpha and beta"));
        assert!(!rule_matches(&rule, "alpha only"));

        let any_rule = RemoteRule {
            match_mode: "any".into(),
            ..rule.clone()
        };
        assert!(rule_matches(&any_rule, "alpha only"));
    }
}
