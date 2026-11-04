use crate::config;
use crate::download;
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

const PKG_URL: &str =
    "https://github.com/burningtnt/Terracotta/releases/download/v0.4.2/terracotta-0.4.2-windows-x86_64-pkg.tar.gz";
const PKG_SHA1: &str = "4d634eff075da6cf86514ec88f34168620b60991";
const PKG_EXE: &str = "terracotta-0.4.2-windows-x86_64.exe";
const BIN_NAME: &str = "terracotta.exe";

#[derive(Debug, Clone, Serialize, Default)]
pub struct TcStatus {
    pub running: bool,
    pub port: u16,
    pub version: String,
    pub error: Option<String>,
    pub state: Option<serde_json::Value>,
}

struct TcRuntime {
    port: u16,
    version: String,
    client: reqwest::Client,
    /// 进程由本实例启动；复用了其他启动器已在跑的实例时为 false，
    /// 退出时不能发 /panic，否则会把别人的房间一起关掉
    owned: bool,
    /// 引导进程句柄，仅用于超时兜底与回收
    child: Option<Child>,
}

static SLOT: OnceLock<Mutex<Option<TcRuntime>>> = OnceLock::new();

fn slot() -> &'static Mutex<Option<TcRuntime>> {
    SLOT.get_or_init(|| Mutex::new(None))
}

/// 串行化启动流程，避免并发调用重复下载/启动两个进程
fn prepare_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn terracotta_dir() -> PathBuf {
    config::data_dir().join("terracotta")
}

fn local_client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(4))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

fn hide_console(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
}

async fn ensure_binary(client: &reqwest::Client) -> Result<PathBuf> {
    let dir = terracotta_dir();
    std::fs::create_dir_all(&dir)?;
    let target = dir.join(BIN_NAME);
    if std::fs::metadata(&target)
        .map(|meta| meta.len() > 1024 * 1024)
        .unwrap_or(false)
    {
        return Ok(target);
    }

    let bytes = download::get_bytes(client, PKG_URL, false)
        .await
        .context("下载陶瓦联机组件失败，请检查网络后重试")?;
    let actual = format!("{:x}", Sha1::digest(&bytes));
    if !actual.eq_ignore_ascii_case(PKG_SHA1) {
        bail!("陶瓦联机组件校验失败，文件可能已损坏，请稍后重试");
    }
    let pkg = dir.join("terracotta-pkg.tar.gz");
    std::fs::write(&pkg, &bytes)?;

    let mut tar = Command::new("tar");
    tar.current_dir(&dir)
        .args(["-xzf", "terracotta-pkg.tar.gz", "-C", "."])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_console(&mut tar);
    let status = tar.status().context("调用 tar 解压失败")?;
    if !status.success() {
        bail!("解压陶瓦联机组件失败");
    }

    let extracted = dir.join(PKG_EXE);
    if !extracted.exists() {
        bail!("陶瓦联机组件缺少可执行文件");
    }
    if target.exists() {
        std::fs::remove_file(&target)?;
    }
    std::fs::rename(&extracted, &target)?;
    let _ = std::fs::remove_file(&pkg);
    Ok(target)
}

fn start_process(exe: &Path) -> Result<(u16, Child)> {
    let dir = terracotta_dir();
    let port_file = dir.join("port.json");
    let _ = std::fs::remove_file(&port_file);

    let mut command = Command::new(exe);
    command
        .arg("--hmcl")
        .arg(&port_file)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_console(&mut command);
    let mut child = command.spawn().context("启动陶瓦联机组件失败")?;

    for _ in 0..120 {
        if let Ok(text) = std::fs::read_to_string(&port_file) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(port) = value.get("port").and_then(|port| port.as_u64()) {
                    if port > 0 && port < 65536 {
                        return Ok((port as u16, child));
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    bail!("陶瓦联机组件启动超时")
}

#[derive(Deserialize)]
struct Meta {
    #[serde(default)]
    version: String,
}

async fn fetch_state(port: u16, client: &reqwest::Client) -> Option<serde_json::Value> {
    client
        .get(format!("http://127.0.0.1:{port}/state"))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .await
        .ok()
}

fn current() -> Result<(u16, reqwest::Client)> {
    let guard = slot().lock().unwrap();
    let runtime = guard
        .as_ref()
        .ok_or_else(|| anyhow!("陶瓦联机尚未启动"))?;
    Ok((runtime.port, runtime.client.clone()))
}

/// 陶瓦自身在系统临时目录写锁文件，内容为 2 字节大端端口号。
/// 用来判断启动前是否已有别的启动器实例在跑。
fn lock_port() -> Option<u16> {
    let path = std::env::temp_dir()
        .join("terracotta")
        .join("terracotta.lock");
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() < 2 {
        return None;
    }
    let port = u16::from_be_bytes([bytes[0], bytes[1]]);
    (port > 0).then_some(port)
}

/// 探测组件是否真的活着；本地回环请求很快，连续三次失败才判定为不可用，
/// 避免系统瞬时卡顿被误判成进程死亡而重启房间
async fn reachable(port: u16) -> bool {
    let probe = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_millis(1500))
        .build()
        .unwrap_or_else(|_| local_client());
    for _ in 0..3 {
        if fetch_state(port, &probe).await.is_some() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    false
}

/// 首次使用时下载并启动陶瓦联机组件，随后返回当前状态。
pub async fn prepare(client: &reqwest::Client) -> Result<TcStatus> {
    let _guard = prepare_lock().lock().await;

    if let Ok((port, _http)) = current() {
        if reachable(port).await {
            return Ok(status().await);
        }
        // 组件已退出或卡死：清掉死槽位，走一次真正的重启
        *slot().lock().unwrap() = None;
    }

    // 启动前已有别的启动器在跑陶瓦时，本实例只是附属进程，
    // 退出时不能发 /panic，否则会把别人的房间一起关掉
    let shared = match lock_port() {
        Some(port) => reachable(port).await,
        None => false,
    };

    let exe = ensure_binary(client).await?;
    let (port, child) = start_process(&exe)?;
    let http = local_client();
    let version = match http
        .get(format!("http://127.0.0.1:{port}/meta"))
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => response
            .json::<Meta>()
            .await
            .map(|meta| meta.version)
            .unwrap_or_default(),
        _ => String::new(),
    };

    {
        let mut guard = slot().lock().unwrap();
        *guard = Some(TcRuntime {
            port,
            version,
            client: http,
            owned: !shared,
            child: Some(child),
        });
    }
    Ok(status().await)
}

pub async fn status() -> TcStatus {
    let current = { current() };
    let Ok((port, client)) = current else {
        return TcStatus::default();
    };
    let version = slot()
        .lock()
        .unwrap()
        .as_ref()
        .map(|runtime| runtime.version.clone())
        .unwrap_or_default();
    let state = fetch_state(port, &client).await;
    TcStatus {
        running: true,
        port,
        version,
        error: if state.is_none() {
            Some("无法读取陶瓦联机状态".into())
        } else {
            None
        },
        state,
    }
}

async fn call(path: &str, query: &[(&str, String)]) -> Result<reqwest::StatusCode> {
    let (port, client) = current()?;
    let mut url = reqwest::Url::parse(&format!("http://127.0.0.1:{port}{path}"))?;
    if !query.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in query {
            if !value.trim().is_empty() {
                pairs.append_pair(key, value);
            }
        }
    }
    let response = client.get(url).send().await.context("请求陶瓦联机失败")?;
    Ok(response.status())
}

/// 解析设置里的公共节点：逗号、中文逗号、空格、换行分隔，去重保序。
/// 只接受 http(s) 地址，避免把误输入的普通文字传给陶瓦。
pub fn parse_nodes(raw: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    raw.split(|c: char| c == ',' || c == '，' || c.is_whitespace())
        .map(str::trim)
        .filter(|item| item.starts_with("http://") || item.starts_with("https://"))
        .filter(|item| seen.insert(item.to_string()))
        .map(str::to_string)
        .collect()
}

pub async fn host(room: Option<String>, player: Option<String>, nodes: &[String]) -> Result<()> {
    let mut query: Vec<(&str, String)> = Vec::new();
    if let Some(room) = room.filter(|value| !value.trim().is_empty()) {
        query.push(("room", room));
    }
    if let Some(player) = player.filter(|value| !value.trim().is_empty()) {
        query.push(("player", player));
    }
    for node in nodes {
        query.push(("public_nodes", node.clone()));
    }
    let status = call("/state/scanning", &query).await?;
    if !status.is_success() {
        bail!("陶瓦联机返回 {status}");
    }
    Ok(())
}

pub async fn join(room: String, player: Option<String>, nodes: &[String]) -> Result<()> {
    let room = room.trim();
    if room.is_empty() {
        bail!("请输入房间邀请码");
    }
    let mut query: Vec<(&str, String)> = vec![("room", room.to_string())];
    if let Some(player) = player.filter(|value| !value.trim().is_empty()) {
        query.push(("player", player));
    }
    for node in nodes {
        query.push(("public_nodes", node.clone()));
    }
    let status = call("/state/guesting", &query).await?;
    if status == reqwest::StatusCode::BAD_REQUEST {
        bail!("邀请码无效，或当前状态无法加入房间");
    }
    if !status.is_success() {
        bail!("陶瓦联机返回 {status}");
    }
    Ok(())
}

pub async fn reset() -> Result<()> {
    let status = call("/state/ide", &[]).await?;
    if !status.is_success() {
        bail!("陶瓦联机返回 {status}");
    }
    Ok(())
}

pub async fn shutdown() -> Result<()> {
    let runtime = { slot().lock().unwrap().take() };
    if let Some(runtime) = runtime {
        if runtime.owned {
            let _ = runtime
                .client
                .get(format!("http://127.0.0.1:{}/panic?peaceful=true", runtime.port))
                .send()
                .await;
        }
        // 引导进程通常已经自行退出，这里只是回收句柄
        if let Some(mut child) = runtime.child {
            for _ in 0..10 {
                if child.try_wait().ok().flatten().is_some() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
    Ok(())
}

/// 应用退出时同步通知陶瓦组件关闭，避免房间残留在后台。
/// 复用了其他启动器实例时不动它，避免误关别人的房间。
pub fn shutdown_blocking() {
    let runtime = { slot().lock().unwrap().take() };
    if let Some(runtime) = runtime {
        if runtime.owned {
            let _ = raw_get(runtime.port, "/panic?peaceful=true");
        }
    }
}

fn raw_get(port: u16, path: &str) -> std::io::Result<()> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
    stream.write_all(
        format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").as_bytes(),
    )?;
    let mut buffer = [0u8; 128];
    let _ = stream.read(&mut buffer);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_public_nodes() {
        let nodes = parse_nodes(
            "https://etnode.example.org/node1, https://etnode.example.org/node2\nhttps://etnode.example.org/node1 随便写的文字",
        );
        assert_eq!(nodes.len(), 2);
        assert!(nodes[0].ends_with("node1"));
        assert!(nodes[1].ends_with("node2"));
        assert!(parse_nodes("").is_empty());
        assert!(parse_nodes("不是地址").is_empty());
        let cn = parse_nodes("https://a.example.org，https://b.example.org");
        assert_eq!(cn.len(), 2);
    }

    #[tokio::test]
    #[ignore]
    async fn prepare_real_terracotta() {
        let client = crate::http::client();
        let initial = prepare(&client).await.expect("准备陶瓦联机失败");
        println!("{}", serde_json::to_string_pretty(&initial).unwrap_or_default());
        assert!(initial.running);
        let state = initial.state.clone().expect("缺少 state");
        assert!(state.get("state").is_some(), "state 结构异常：{state}");

        host(None, Some("DOCL Test".into()), &[])
            .await
            .expect("创建房间请求失败");
        let mut scanning = false;
        for _ in 0..30 {
            tokio::time::sleep(Duration::from_millis(200)).await;
            let current = status().await;
            let name = current
                .state
                .as_ref()
                .and_then(|state| state.get("state"))
                .and_then(|state| state.as_str())
                .unwrap_or("");
            if name == "host-scanning" {
                scanning = true;
                break;
            }
            if name == "exception" {
                panic!("创建房间进入异常：{:?}", current.state);
            }
        }
        reset().await.ok();
        shutdown().await.ok();
        assert!(scanning, "未进入 host-scanning 状态");
    }
}
