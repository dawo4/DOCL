use crate::mirror;
use anyhow::{anyhow, bail, Context, Result};
use futures::stream::{FuturesUnordered, StreamExt};
use serde::Serialize;
use sha1::{Digest, Sha1};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncWriteExt;

#[derive(Clone, Debug)]
pub struct FileTask {
    pub url: String,
    pub dest: PathBuf,
    pub sha1: Option<String>,
    pub size: u64,
    pub label: String,
}

#[derive(Debug)]
pub struct ProgressState {
    pub stage: String,
    message: Mutex<String>,
    /// 依次经过的安装阶段，供下载面板显示成步骤列表
    steps: Mutex<Vec<ProgressStep>>,
    pub files_done: AtomicU64,
    pub files_total: AtomicU64,
    pub bytes_done: AtomicU64,
    pub bytes_total: AtomicU64,
    pub cancel: AtomicBool,
    pub active: AtomicBool,
    /// 是否已有刷新联动的任务，避免重复起多个
    ticking: AtomicBool,
    /// 当前阶段的计时基准，进入新阶段会重置
    started: Mutex<Instant>,
}

/// 面板最多展示的步骤数量，避免加载器日志刷屏
const MAX_STEPS: usize = 8;

#[derive(Clone, Debug, Serialize)]
pub struct ProgressStep {
    pub label: String,
    pub done: bool,
}

#[derive(Clone, Serialize)]
pub struct ProgressSnapshot {
    pub stage: String,
    pub message: String,
    pub steps: Vec<ProgressStep>,
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub elapsed_ms: u64,
    pub active: bool,
}

impl ProgressState {
    pub fn new(stage: &str) -> Arc<Self> {
        Arc::new(Self {
            stage: stage.to_string(),
            message: Mutex::new(String::new()),
            steps: Mutex::new(Vec::new()),
            files_done: AtomicU64::new(0),
            files_total: AtomicU64::new(0),
            bytes_done: AtomicU64::new(0),
            bytes_total: AtomicU64::new(0),
            cancel: AtomicBool::new(false),
            active: AtomicBool::new(true),
            ticking: AtomicBool::new(false),
            started: Mutex::new(Instant::now()),
        })
    }

    /// 进入新的下载阶段：重置本阶段计数与计时基准。
    /// 不能只覆盖 total 而不清 done，否则第二阶段会出现「已下载 885MB / 总量 9MB」，
    /// 百分比直接算成 100%、剩余文件变 0，看起来像卡死。
    pub fn begin_phase(&self, files_total: u64, bytes_total: u64) {
        self.files_done.store(0, Ordering::Relaxed);
        self.bytes_done.store(0, Ordering::Relaxed);
        self.files_total.store(files_total, Ordering::Relaxed);
        self.bytes_total.store(bytes_total, Ordering::Relaxed);
        if let Ok(mut started) = self.started.lock() {
            *started = Instant::now();
        }
    }

    pub fn set_message(&self, msg: impl Into<String>) {
        let msg = msg.into();
        if msg.trim().is_empty() {
            return;
        }
        if let Ok(mut steps) = self.steps.lock() {
            let repeated = steps
                .last()
                .map(|step| step.label == msg)
                .unwrap_or(false);
            if !repeated {
                if let Some(last) = steps.last_mut() {
                    last.done = true;
                }
                steps.push(ProgressStep {
                    label: msg.clone(),
                    done: false,
                });
                if steps.len() > MAX_STEPS {
                    steps.remove(0);
                }
            }
        }
        if let Ok(mut guard) = self.message.lock() {
            *guard = msg;
        }
    }

    /// 只更新当前阶段的细节（安装器实时输出），不新增步骤。
    pub fn set_detail(&self, msg: impl Into<String>) {
        let msg = msg.into();
        if msg.trim().is_empty() {
            return;
        }
        if let Ok(mut steps) = self.steps.lock() {
            if let Some(last) = steps.last_mut() {
                if !last.done {
                    last.label = msg.clone();
                }
            }
        }
        if let Ok(mut guard) = self.message.lock() {
            *guard = msg;
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn snapshot(&self) -> ProgressSnapshot {
        let active = self.active.load(Ordering::Relaxed);
        let mut steps = self
            .steps
            .lock()
            .map(|list| list.clone())
            .unwrap_or_default();
        // 任务结束后最后一步也视为完成
        if !active {
            if let Some(last) = steps.last_mut() {
                last.done = true;
            }
        }
        ProgressSnapshot {
            stage: self.stage.clone(),
            message: self
                .message
                .lock()
                .map(|m| m.clone())
                .unwrap_or_default(),
            steps,
            files_done: self.files_done.load(Ordering::Relaxed),
            files_total: self.files_total.load(Ordering::Relaxed),
            bytes_done: self.bytes_done.load(Ordering::Relaxed),
            bytes_total: self.bytes_total.load(Ordering::Relaxed),
            elapsed_ms: self
                .started
                .lock()
                .map(|started| started.elapsed().as_millis() as u64)
                .unwrap_or(0),
            active,
        }
    }
}

pub fn emit_now(app: &AppHandle, progress: &Arc<ProgressState>) {
    let _ = app.emit("install-progress", progress.snapshot());
}

fn start_ticker(app: AppHandle, progress: Arc<ProgressState>) {
    // 已经在刷新就不重复起任务；任务结束后由本轮循环复位
    if progress.ticking.swap(true, Ordering::Relaxed) {
        return;
    }
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(180)).await;
            let _ = app.emit("install-progress", progress.snapshot());
            if !progress.active.load(Ordering::Relaxed) {
                break;
            }
        }
        progress.ticking.store(false, Ordering::Relaxed);
    });
}

/// 进入一个没有逐文件进度的阶段：更新文案、保持界面刷新。
pub fn begin_stage(app: &AppHandle, progress: &Arc<ProgressState>, message: impl Into<String>) {
    progress.set_message(message);
    progress.active.store(true, Ordering::Relaxed);
    start_ticker(app.clone(), progress.clone());
    emit_now(app, progress);
}

pub async fn get_bytes(
    client: &reqwest::Client,
    url: &str,
    use_mirror: bool,
) -> Result<Vec<u8>> {
    let mut last_err: Option<anyhow::Error> = None;
    let mirror_url = mirror::rewrite(url);
    // 镜像未覆盖该主机时 rewrite 原样返回，去重避免对同一地址打两遍
    let raw = if use_mirror {
        vec![mirror_url, url.to_string()]
    } else {
        vec![url.to_string(), mirror_url]
    };
    let mut seen = std::collections::HashSet::new();
    let candidates: Vec<String> = raw.into_iter().filter(|c| seen.insert(c.clone())).collect();
    // 单文件请求必须有总超时：connect_timeout 只管握手，
    // 响应体挂起（官方源被墙断流）会无限等待，表现为「卡在某一步」
    for (idx, candidate) in candidates.iter().enumerate() {
        for attempt in 0..2 {
            let request = client
                .get(candidate)
                .timeout(Duration::from_secs(45));
            match request.send().await {
                Ok(resp) if resp.status().is_success() => {
                    match resp.bytes().await {
                        Ok(bytes) => return Ok(bytes.to_vec()),
                        Err(err) => last_err = Some(err.into()),
                    }
                }
                Ok(resp) => last_err = Some(anyhow!("HTTP {}", resp.status())),
                Err(err) => last_err = Some(err.into()),
            }
            // 首次失败稍等再重试
            if attempt == 0 {
                tokio::time::sleep(Duration::from_millis(400)).await;
            }
        }
        // 还有下一个候选时才等待，避免末尾白等
        if idx + 1 < candidates.len() {
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("网络请求失败：{url}")))
}

pub async fn get_json<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
    use_mirror: bool,
) -> Result<T> {
    let bytes = get_bytes(client, url, use_mirror).await?;
    serde_json::from_slice(&bytes).context("解析 JSON 失败")
}

/// 等待取消标志被置位，供 select 分支使用
async fn wait_cancel(progress: &Arc<ProgressState>) {
    while !progress.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(80)).await;
    }
}

/// 取消标志存在时才等待，否则永不返回（select 分支保持静默）。
async fn wait_cancel_opt(progress: Option<&Arc<ProgressState>>) {
    match progress {
        Some(progress) => wait_cancel(progress).await,
        None => std::future::pending::<()>().await,
    }
}

/// 竞速用的单次请求：不重试、不走候选列表，避免与竞速层叠加放大请求量。
async fn get_bytes_once(client: &reqwest::Client, url: String) -> Result<Vec<u8>> {
    let resp = client
        .get(&url)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .with_context(|| format!("请求失败：{url}"))?;
    if !resp.status().is_success() {
        bail!("HTTP {}", resp.status());
    }
    Ok(resp.bytes().await.context("读取响应失败")?.to_vec())
}

/// 竞速请求：先发一路，超过 `spawn_delay_ms` 没回再补下一路（轮流走镜像/官方），
/// 谁先成功用谁。只用于版本清单、资源索引这类关键小请求；大文件竞速会成倍浪费带宽。
/// 补位间隔要按体积给：小 JSON 250ms，几 MB 的索引应放宽，否则正常网络也会补位。
pub async fn get_bytes_race(
    client: &reqwest::Client,
    url: &str,
    use_mirror: bool,
    progress: Option<&Arc<ProgressState>>,
    lanes: usize,
    spawn_delay_ms: u64,
) -> Result<Vec<u8>> {
    let lanes = lanes.clamp(1, 4);
    let mirror_url = mirror::rewrite(url);
    let ordered = if use_mirror {
        vec![mirror_url, url.to_string()]
    } else {
        vec![url.to_string(), mirror_url]
    };
    let mut seen = std::collections::HashSet::new();
    let targets: Vec<String> = ordered.into_iter().filter(|c| seen.insert(c.clone())).collect();

    let mut pending = FuturesUnordered::new();
    let mut started = 0usize;
    let mut last_err: Option<anyhow::Error> = None;

    loop {
        if started < lanes {
            let target = targets[started % targets.len()].clone();
            pending.push(get_bytes_once(client, target));
            started += 1;
        }
        if pending.is_empty() {
            break;
        }
        let can_spawn = started < lanes;
        tokio::select! {
            biased;
            _ = wait_cancel_opt(progress) => bail!("下载已取消"),
            result = pending.next() => {
                match result {
                    Some(Ok(bytes)) => return Ok(bytes),
                    Some(Err(err)) => last_err = Some(err),
                    None => {}
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(spawn_delay_ms)), if can_spawn => {}
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("网络请求失败：{url}")))
}

/// 竞速获取并解析 JSON。
pub async fn get_json_race<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
    use_mirror: bool,
    progress: Option<&Arc<ProgressState>>,
) -> Result<T> {
    let bytes = get_bytes_race(client, url, use_mirror, progress, 3, 250).await?;
    serde_json::from_slice(&bytes).context("解析 JSON 失败")
}

/* ---------- 全局限速 ---------- */

/// 全局下载限速（字节/秒），0 表示不限速；由设置页通过 set_speed_limit 更新
static SPEED_LIMIT: AtomicU64 = AtomicU64::new(0);
static WINDOW_BYTES: AtomicU64 = AtomicU64::new(0);
static WINDOW_START: Mutex<Option<Instant>> = Mutex::new(None);

pub fn set_speed_limit(bytes_per_sec: u64) {
    SPEED_LIMIT.store(bytes_per_sec, Ordering::Relaxed);
}

/// 用户是否强制「仅镜像」：此时不看智能排序，全部镜像优先
static FORCE_MIRROR: AtomicBool = AtomicBool::new(false);

pub fn set_force_mirror(value: bool) {
    FORCE_MIRROR.store(value, Ordering::Relaxed);
}

/// 滑动 1 秒窗口的简单限速：所有并发任务共享同一个配额。
/// 超出配额时挂起等待到下一窗口，保证平均速率不超过限制。
async fn throttle(bytes: u64) {
    let limit = SPEED_LIMIT.load(Ordering::Relaxed);
    // 未设限，或单块就超过整秒配额时直接放行，否则永远等不到够用的窗口
    if limit == 0 || bytes > limit {
        return;
    }
    loop {
        let now = Instant::now();
        let window_start = {
            let mut guard = WINDOW_START.lock().unwrap();
            match *guard {
                Some(start) if now.duration_since(start).as_millis() < 1000 => start,
                _ => {
                    *guard = Some(now);
                    WINDOW_BYTES.store(0, Ordering::Relaxed);
                    now
                }
            }
        };
        let elapsed = now.duration_since(window_start).as_millis() as u64;
        let used = WINDOW_BYTES.fetch_add(bytes, Ordering::Relaxed) + bytes;
        if used <= limit {
            return;
        }
        // 配额不足：退回记账，睡到本窗口结束再试。
        // 窗口可能已被其他任务重置，用饱和减法避免 u64 下溢成巨大值
        let _ = WINDOW_BYTES.try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            Some(value.saturating_sub(bytes))
        });
        let remain_ms = 1000u64.saturating_sub(elapsed).max(10);
        tokio::time::sleep(Duration::from_millis(remain_ms)).await;
    }
}

/// 镜像速度下限（字节/秒）：低于它在探测窗口后判定源不合格并换源，PCL 同样用 256 KB/s
const SLOW_SOURCE_BYTES_PER_SEC: u64 = 256 * 1024;
/// 速度探测窗口，太短容易被网络抖动误判
const SLOW_PROBE_WINDOW: Duration = Duration::from_secs(6);

/// 临时性失败（5xx / 超时）：换源并等待后仍有较大概率成功，
/// 退避时间比 4xx 长，避免服务端短暂不可用时三次连撞。
#[derive(Debug)]
struct TransientError {
    message: String,
    /// 服务端 Retry-After（秒），没有则为 None
    retry_after: Option<Duration>,
}

impl std::fmt::Display for TransientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TransientError {}

/// 统计本次尝试已经计入进度的字节数。
/// 失败时（未 commit）自动扣回，避免重试把同一份数据重复累加导致进度虚高。
struct ByteTracker<'a> {
    progress: &'a ProgressState,
    written: u64,
    committed: bool,
}

impl<'a> ByteTracker<'a> {
    fn new(progress: &'a ProgressState) -> Self {
        Self {
            progress,
            written: 0,
            committed: false,
        }
    }

    fn add(&mut self, bytes: u64) {
        self.written += bytes;
        self.progress
            .bytes_done
            .fetch_add(bytes, Ordering::Relaxed);
    }

    #[cfg(test)]
    fn written(&self) -> u64 {
        self.written
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for ByteTracker<'_> {
    fn drop(&mut self) {
        if !self.committed && self.written > 0 {
            self.progress
                .bytes_done
                .fetch_sub(self.written, Ordering::Relaxed);
        }
    }
}

/// 可被取消打断的等待：长退避期间用户点停止不必干等到超时。
async fn sleep_cancellable(progress: &ProgressState, total: Duration) -> bool {
    let step = Duration::from_millis(200);
    let mut left = total;
    while left > Duration::ZERO {
        if progress.is_cancelled() {
            return true;
        }
        let chunk = left.min(step);
        tokio::time::sleep(chunk).await;
        left -= chunk;
    }
    progress.is_cancelled()
}


/// 发起一次下载请求（只看连接与响应头），供单源与双源竞速共用。
async fn open_stream(
    client: &reqwest::Client,
    url: &str,
    label: &str,
    resume_from: u64,
) -> Result<reqwest::Response> {
    let mut request = client.get(url);
    if resume_from > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={resume_from}-"));
    }
    // 连接阶段必须有超时：CDN 握手挂起时 send() 会一直等，取消也无法打断
    let resp = match tokio::time::timeout(Duration::from_secs(10), request.send()).await {
        Ok(result) => result.with_context(|| format!("连接失败：{label}"))?,
        Err(_) => {
            return Err(anyhow::Error::new(TransientError {
                message: format!("连接超时：{label}"),
                retry_after: None,
            }))
        }
    };
    if !resp.status().is_success() {
        let status = resp.status();
        // 5xx 与 429 是临时不可用，等一会儿换源还有机会；4xx 直接判定失败
        if status.is_server_error() || status.as_u16() == 429 {
            let retry_after = resp
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.trim().parse::<u64>().ok())
                .map(Duration::from_secs)
                // 服务端偶有给出很大的值，超过 20 秒就不等了，直接按退避表走
                .filter(|wait| *wait <= Duration::from_secs(20));
            return Err(anyhow::Error::new(TransientError {
                message: format!("服务器返回 {status}"),
                retry_after,
            }));
        }
        bail!("服务器返回 {}", status);
    }
    Ok(resp)
}

/// 首选源 2 秒没拿到响应时，并发试另一个源，先连上的算赢。
/// 竞速只发生在连接/响应头阶段：输的一方还没开始写文件，丢弃即可。
async fn open_hedged(
    client: &reqwest::Client,
    primary: &str,
    alternate: Option<&str>,
    label: &str,
    resume_from: u64,
) -> Result<(reqwest::Response, String)> {
    let mut primary_fut = Box::pin(open_stream(client, primary, label, resume_from));
    match tokio::time::timeout(Duration::from_secs(2), &mut primary_fut).await {
        Ok(result) => result.map(|resp| (resp, primary.to_string())),
        Err(_) => {
            let Some(alternate) = alternate else {
                return primary_fut
                    .await
                    .map(|resp| (resp, primary.to_string()));
            };
            let mut alternate_fut = Box::pin(open_stream(client, alternate, label, resume_from));
            tokio::select! {
                result = &mut primary_fut => result.map(|resp| (resp, primary.to_string())),
                result = &mut alternate_fut => result.map(|resp| (resp, alternate.to_string())),
            }
        }
    }
}

/// 断点续传时先把已有字节喂给 SHA1，保证最终校验覆盖整份文件
pub(crate) async fn hash_existing(path: &Path, hasher: &mut Sha1) -> Result<()> {
    use tokio::io::AsyncReadExt;
    let mut file = tokio::fs::File::open(path).await?;
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(())
}

async fn fetch_to_file(
    client: &reqwest::Client,
    primary: &str,
    alternate: Option<&str>,
    task: &FileTask,
    progress: &Arc<ProgressState>,
    allow_slow_bail: bool,
) -> Result<()> {
    if let Some(parent) = task.dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    // 与 download_stream 保持一致：用 `文件名.part` 而不是替换扩展名，
    // 否则同目录下 `a.jar` 与 `a.zip` 会共用同一个断点文件
    let tmp = PathBuf::from(format!("{}.part", task.dest.display()));
    // 断点续传：保留上次中断的 .part，网络恢复后从已有字节继续
    let mut resume_from = tokio::fs::metadata(&tmp)
        .await
        .map(|meta| meta.len())
        .unwrap_or(0);
    if task.size > 0 && resume_from >= task.size {
        // 上次可能已经下完、只是没来得及改名：校验通过就直接落盘，避免整份重下
        let complete = resume_from == task.size
            && match task.sha1.as_deref().filter(|value| !value.is_empty()) {
                Some(expected) => {
                    let mut hasher = Sha1::new();
                    hash_existing(&tmp, &mut hasher).await.is_ok()
                        && format!("{:x}", hasher.finalize()).eq_ignore_ascii_case(expected)
                }
                None => true,
            };
        if complete {
            if task.dest.exists() {
                let _ = tokio::fs::remove_file(&task.dest).await;
            }
            tokio::fs::rename(&tmp, &task.dest).await?;
            // 字节数在这里补记（正常下载由 ByteTracker 记）；文件数由 download_one 统一累加
            progress
                .bytes_done
                .fetch_add(resume_from, Ordering::Relaxed);
            return Ok(());
        }
        let _ = tokio::fs::remove_file(&tmp).await;
        resume_from = 0;
    }

    let (resp, used_url) =
        match open_hedged(client, primary, alternate, &task.label, resume_from).await {
            Ok(value) => value,
            Err(err) if resume_from > 0 => {
                // 服务端可能不认这个 Range（416 等），清掉已有的 part 重头来
                let _ = tokio::fs::remove_file(&tmp).await;
                match open_hedged(client, primary, alternate, &task.label, 0).await {
                    Ok(value) => value,
                    Err(_) => return Err(err),
                }
            }
            Err(err) => return Err(err),
        };
    #[cfg(debug_assertions)]
    if used_url != primary {
        eprintln!("[download] {} 竞速改用 {}", task.label, used_url);
    }

    let append = resume_from > 0 && resp.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    if !append {
        resume_from = 0;
    }
    let mut file = if append {
        tokio::fs::OpenOptions::new().append(true).open(&tmp).await?
    } else {
        tokio::fs::File::create(&tmp).await?
    };
    let mut hasher = Sha1::new();
    let mut tracker = ByteTracker::new(progress.as_ref());
    if append {
        hash_existing(&tmp, &mut hasher).await?;
        tracker.add(resume_from);
    }
    let mut stream = resp.bytes_stream();
    // 连接已完成，从这里开始算有效速度
    let probe_start = Instant::now();
    let mut probe_bytes = 0u64;
    let mut probed = false;
    loop {
        if progress.is_cancelled() {
            // 保留 .part，下次重试可以续传
            drop(file);
            bail!("下载已取消");
        }
        // 单块读取加超时，避免数据流卡住时永远等不到下一个 chunk
        let next = match tokio::time::timeout(Duration::from_secs(15), stream.next()).await {
            Ok(value) => value,
            Err(_) => {
                drop(file);
                return Err(anyhow::Error::new(TransientError {
                    message: format!("下载超时（15 秒无数据）：{}", task.label),
                    retry_after: None,
                }));
            }
        };
        let Some(chunk) = next else { break };
        // 流中途断开（连接重置等）也属于临时性失败，值得换源重试
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(err) => {
                drop(file);
                return Err(anyhow::Error::new(TransientError {
                    message: format!("读取下载数据失败：{err}"),
                    retry_after: None,
                }));
            }
        };
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        tracker.add(chunk.len() as u64);
        throttle(chunk.len() as u64).await;

        // 下载源慢到不可用时提前放弃，交给上层换源（用户主动限速时不判定，否则必然误切）
        if allow_slow_bail && !probed {
            probe_bytes += chunk.len() as u64;
            let elapsed = probe_start.elapsed();
            if elapsed >= SLOW_PROBE_WINDOW {
                probed = true;
                let speed = probe_bytes as f64 / elapsed.as_secs_f64();
                if SPEED_LIMIT.load(Ordering::Relaxed) == 0
                    && speed < SLOW_SOURCE_BYTES_PER_SEC as f64
                {
                    // 保留已下载的 .part，换源后从断点继续
                    drop(file);
                    // 丢弃的数据由 ByteTracker 在返回时统一扣回
                    bail!(
                        "下载源速度过慢（{} KB/s）：{}，换源重试",
                        (speed / 1024.0) as u64,
                        task.label
                    );
                }
            }
        }
    }
    file.flush().await?;
    drop(file);

    let actual = format!("{:x}", hasher.finalize());
    // 续传时以磁盘上的真实长度为准（tracker 也包含续传的字节，但磁盘更可靠）
    let written_total = tokio::fs::metadata(&tmp)
        .await
        .map(|meta| meta.len())
        .unwrap_or(0);
    // SHA1 是权威校验；没有哈希时才用清单长度兜底，
    // 避免个别清单里的 size 写错导致正确文件被误判失败
    match task.sha1.as_deref().filter(|value| !value.is_empty()) {
        Some(expected) => {
            if !expected.eq_ignore_ascii_case(&actual) {
                let _ = tokio::fs::remove_file(&tmp).await;
                bail!("文件校验失败（SHA1 不匹配）：{}", task.label);
            }
        }
        None => {
            if task.size > 0 && written_total != task.size {
                let _ = tokio::fs::remove_file(&tmp).await;
                bail!(
                    "文件大小不符（预期 {} 字节，实际 {}）：{}",
                    task.size,
                    written_total,
                    task.label
                );
            }
        }
    }
    if task.dest.exists() {
        let _ = tokio::fs::remove_file(&task.dest).await;
    }
    tokio::fs::rename(&tmp, &task.dest).await?;
    tracker.commit();
    Ok(())
}

/// 单文件流式下载：支持 Range 续传、连接/停滞超时，并在候选地址间换源重试。
/// 用于启动器更新、单个 Mod 更新这类不走批量下载面板的场景，
/// 避免它们各自实现一套「无超时、无续传」的循环而卡死。
pub async fn download_stream(
    client: &reqwest::Client,
    urls: &[String],
    dest: &Path,
    on_progress: &mut (dyn FnMut(u64, u64) + Send),
) -> Result<()> {
    if urls.is_empty() {
        bail!("缺少下载地址");
    }
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let part = PathBuf::from(format!("{}.part", dest.display()));
    let mut last_err: Option<anyhow::Error> = None;

    for attempt in 0..2 {
        for url in urls {
            let resume_from = tokio::fs::metadata(&part)
                .await
                .map(|meta| meta.len())
                .unwrap_or(0);
            let mut request = client.get(url);
            if resume_from > 0 {
                request = request.header(reqwest::header::RANGE, format!("bytes={resume_from}-"));
            }
            let resp = match tokio::time::timeout(Duration::from_secs(10), request.send()).await {
                Ok(Ok(resp)) => resp,
                Ok(Err(err)) => {
                    last_err = Some(err.into());
                    continue;
                }
                Err(_) => {
                    last_err = Some(anyhow!("连接超时：{url}"));
                    continue;
                }
            };
            // 416 说明 .part 已经到文件末尾或旧分片失效。保留它只会每轮都 416，
            // 清掉后下一轮会从头下载
            if resp.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
                let _ = tokio::fs::remove_file(&part).await;
                last_err = Some(anyhow!("断点分片已失效，已清除并重新下载"));
                continue;
            }
            if !resp.status().is_success() {
                last_err = Some(anyhow!("服务器返回 {}", resp.status()));
                continue;
            }
            // 服务端不认 Range 时退回整份重下
            let append = resume_from > 0 && resp.status() == reqwest::StatusCode::PARTIAL_CONTENT;
            let start = if append { resume_from } else { 0 };
            let total = resp
                .content_length()
                .map(|len| len + start)
                .unwrap_or(0);
            let mut file = if append {
                tokio::fs::OpenOptions::new().append(true).open(&part).await?
            } else {
                tokio::fs::File::create(&part).await?
            };
            let mut written = start;
            let mut stream_failed: Option<anyhow::Error> = None;
            let mut stream = resp.bytes_stream();
            loop {
                let next = match tokio::time::timeout(Duration::from_secs(15), stream.next()).await
                {
                    Ok(value) => value,
                    Err(_) => {
                        stream_failed = Some(anyhow!("下载超时（15 秒无数据）"));
                        break;
                    }
                };
                let Some(chunk) = next else { break };
                match chunk {
                    Ok(chunk) => {
                        file.write_all(&chunk).await?;
                        written += chunk.len() as u64;
                        // 与批量下载共用全局限速配额
                        throttle(chunk.len() as u64).await;
                        on_progress(written, total);
                    }
                    Err(err) => {
                        stream_failed = Some(err.into());
                        break;
                    }
                }
            }
            let _ = file.flush().await;
            drop(file);
            if let Some(err) = stream_failed {
                // 保留 .part，下一轮从断点继续
                last_err = Some(err);
                continue;
            }
            if total > 0 && written != total {
                // 长度对不上说明 `.part` 可能是旧版本或旧文件的残留，留着会一直续错，清掉重来
                let _ = tokio::fs::remove_file(&part).await;
                last_err = Some(anyhow!(
                    "文件大小不符（预期 {} 字节，实际 {written}）",
                    total
                ));
                continue;
            }
            if dest.exists() {
                let _ = tokio::fs::remove_file(dest).await;
            }
            tokio::fs::rename(&part, dest).await?;
            return Ok(());
        }
        if attempt == 0 {
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("下载失败：{}", dest.display())))
}

/* ---------- 按 SHA1 的下载缓存 ---------- */

/// 上限 2 GB，超出后按修改时间淘汰最旧的文件
const CACHE_LIMIT_BYTES: u64 = 2 * 1024 * 1024 * 1024;

fn cache_path(sha1: &str) -> PathBuf {
    crate::config::data_dir().join("download-cache").join(sha1)
}

/// 同一个文件在别的实例下过一次就不必重下：优先硬链接，跨盘时退回复制
async fn reuse_from_cache(task: &FileTask, progress: &ProgressState) -> bool {
    let Some(sha1) = task.sha1.as_deref().filter(|value| !value.is_empty()) else {
        return false;
    };
    if progress.is_cancelled() {
        return false;
    }
    let cached = cache_path(sha1);
    let Ok(meta) = tokio::fs::metadata(&cached).await else {
        return false;
    };
    if task.size > 0 && meta.len() != task.size {
        // 缓存可能被原地截断（例如解压覆盖写到了硬链接上），删掉让它重新沉淀
        let _ = tokio::fs::remove_file(&cached).await;
        return false;
    }
    // 缓存文件名就是 SHA1，但硬链接可能被目标文件原地改写；命中后仍校验内容
    let mut hasher = Sha1::new();
    if hash_existing(&cached, &mut hasher).await.is_err()
        || !format!("{:x}", hasher.finalize()).eq_ignore_ascii_case(sha1)
    {
        let _ = tokio::fs::remove_file(&cached).await;
        return false;
    }
    if let Some(parent) = task.dest.parent() {
        if tokio::fs::create_dir_all(parent).await.is_err() {
            return false;
        }
    }
    let _ = tokio::fs::remove_file(&task.dest).await;
    let reused = tokio::fs::hard_link(&cached, &task.dest)
        .await
        .is_ok()
        || tokio::fs::copy(&cached, &task.dest).await.is_ok();
    if !reused {
        return false;
    }
    progress.files_done.fetch_add(1, Ordering::Relaxed);
    progress
        .bytes_done
        .fetch_add(meta.len(), Ordering::Relaxed);
    true
}

async fn store_in_cache(task: &FileTask) {
    let Some(sha1) = task.sha1.as_deref().filter(|value| !value.is_empty()) else {
        return;
    };
    let cached = cache_path(sha1);
    if let Ok(meta) = tokio::fs::metadata(&cached).await {
        // 同 sha1 但长度不对，说明缓存条目已经损坏，替换掉
        if task.size == 0 || meta.len() == task.size {
            return;
        }
        let _ = tokio::fs::remove_file(&cached).await;
    }
    if let Some(parent) = cached.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    if tokio::fs::hard_link(&task.dest, &cached).await.is_ok() {
        return;
    }
    let _ = tokio::fs::copy(&task.dest, &cached).await;
}

/// 每批任务结束后清理一次缓存，避免无限增长
pub async fn prune_download_cache() {
    let dir = crate::config::data_dir().join("download-cache");
    let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
        return;
    };
    let mut files: Vec<(u64, u64, PathBuf)> = Vec::new();
    let mut total = 0u64;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let Ok(meta) = entry.metadata().await else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        total += meta.len();
        let modified = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        files.push((modified, meta.len(), entry.path()));
    }
    if total <= CACHE_LIMIT_BYTES {
        return;
    }
    files.sort_by_key(|(modified, _, _)| *modified);
    let mut left = total;
    for (_, size, path) in files {
        if left <= CACHE_LIMIT_BYTES * 3 / 4 {
            break;
        }
        left = left.saturating_sub(size);
        let _ = tokio::fs::remove_file(&path).await;
    }
}

async fn download_one(
    client: &reqwest::Client,
    task: &FileTask,
    use_mirror: bool,
    mirror_failures: &AtomicU64,
    progress: &Arc<ProgressState>,
) -> Result<()> {
    if progress.is_cancelled() {
        bail!("下载已取消");
    }
    if task.dest.exists() {
        if let Ok(meta) = tokio::fs::metadata(&task.dest).await {
            if task.size == 0 || meta.len() == task.size {
                progress.files_done.fetch_add(1, Ordering::Relaxed);
                progress
                    .bytes_done
                    .fetch_add(meta.len(), Ordering::Relaxed);
                return Ok(());
            }
        }
    }

    // 本地缓存命中就不必再连网
    if reuse_from_cache(task, progress.as_ref()).await {
        return Ok(());
    }

    let mirror_url = mirror::rewrite(&task.url);
    let official_url = task.url.clone();
    let mirror_healthy = use_mirror && mirror_failures.load(Ordering::Relaxed) < 4;
    // 智能排序：Modrinth CDN 国内多数网络不可达，镜像优先；
    // CurseForge 等官方源通常更快，官方优先，慢了再由慢速降级切走。
    // 用户显式选「仅镜像」时不排序，一律镜像优先。
    let mirror_first = mirror_healthy
        && (FORCE_MIRROR.load(Ordering::Relaxed) || mirror::prefer_mirror_first(&task.url));
    let order = if mirror_first {
        [mirror_url, official_url]
    } else {
        [official_url, mirror_url]
    };
    // 两个候选地址不同且镜像可用时，才值得做双源竞速
    let hedge = mirror_healthy && order[0] != order[1];

    let mut last_err: Option<anyhow::Error> = None;
    // 5xx / 超时给服务端留出恢复时间：2s → 6s → 15s，共 4 次尝试
    const ATTEMPTS: usize = 4;
    const BACKOFF_SECS: [u64; 3] = [2, 6, 15];
    for attempt in 0..ATTEMPTS {
        if progress.is_cancelled() {
            bail!("下载已取消");
        }
        let index = attempt % 2;
        let uses_mirror = if mirror_first { index == 0 } else { index == 1 };
        let url = &order[index];
        let alternate = if hedge {
            Some(order[1 - index].as_str())
        } else {
            None
        };
        // 前几次都允许因过慢切源，最后一次只求下完，避免两个源都慢时直接失败
        let allow_slow_bail = attempt + 1 < ATTEMPTS;
        match fetch_to_file(client, url, alternate, task, progress, allow_slow_bail).await {
            Ok(()) => {
                // 镜像侧成功一次就说明源是健康的：清零连续失败计数，
                // 否则几次偶发失败会把整批文件都切成官方优先，反而更慢
                if use_mirror && uses_mirror {
                    mirror_failures.store(0, Ordering::Relaxed);
                }
                // 成功的文件放进缓存，别的实例再装同一个文件就能秒完成
                store_in_cache(task).await;
                progress.files_done.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
            Err(err) => {
                if use_mirror && uses_mirror {
                    mirror_failures.fetch_add(1, Ordering::Relaxed);
                }
                // 临时性失败等久一点，4xx 之类仍按原来的短间隔快速换源
                let wait = match err.downcast_ref::<TransientError>() {
                    Some(transient) => transient.retry_after.unwrap_or_else(|| {
                        Duration::from_secs(BACKOFF_SECS[attempt.min(BACKOFF_SECS.len() - 1)])
                    }),
                    None => Duration::from_millis(400 * (attempt as u64 + 1)),
                };
                last_err = Some(err);
                if attempt + 1 < ATTEMPTS && sleep_cancellable(progress, wait).await {
                    bail!("下载已取消");
                }
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("下载失败：{}", task.label)))
}

/// 补下单个文件：复用换源与重试逻辑（内部最多 3 次），
/// 但用独立的进度对象，不干扰主下载面板。
pub async fn refetch(client: &reqwest::Client, task: &FileTask, use_mirror: bool) -> Result<()> {
    // 损坏文件必须删掉：download_one 对已存在且未给长度的文件会直接跳过
    let _ = tokio::fs::remove_file(&task.dest).await;
    // 缓存条目也可能同样损坏，命中后只会把坏文件重新硬链接回来
    if let Some(sha1) = task.sha1.as_deref().filter(|value| !value.is_empty()) {
        let _ = tokio::fs::remove_file(cache_path(sha1)).await;
    }
    let progress = ProgressState::new("repair");
    let failures = AtomicU64::new(0);
    download_one(client, task, use_mirror, &failures, &progress).await
}

/// 下载任务并发上限。设置页滑条、各调用点都必须引用它，
/// 避免像整合包那样各自硬编码成 32 导致上限不一致。
pub const MAX_CONCURRENCY: usize = 64;

/// 一批下载的结果。个别文件失败不再让整批任务直接报错，
/// 由调用方决定是提示还是中断。
#[derive(Debug, Default)]
pub struct DownloadOutcome {
    /// 失败文件的标签与原因
    pub failed: Vec<(String, String)>,
}

/// 最近一批失败任务，用于「重试失败文件」
#[derive(Clone)]
pub struct FailedBatch {
    pub tasks: Vec<FileTask>,
    pub concurrency: usize,
    pub use_mirror: bool,
}

static FAILED_BATCH: OnceLock<Mutex<Option<FailedBatch>>> = OnceLock::new();

fn failed_batch_slot() -> &'static Mutex<Option<FailedBatch>> {
    FAILED_BATCH.get_or_init(|| Mutex::new(None))
}

fn store_failed_batch(batch: Option<FailedBatch>) {
    let Ok(mut slot) = failed_batch_slot().lock() else {
        return;
    };
    let Some(incoming) = batch else {
        *slot = None;
        return;
    };
    // 同一次作业会分多个批次下载，失败项要合并而不是覆盖；
    // 重试时也可能只剩最后一次的小批次，导致前面的失败文件丢失
    if let Some(existing) = slot.as_mut() {
        let mut dests: std::collections::HashSet<String> = existing
            .tasks
            .iter()
            .map(|task| task.dest.to_string_lossy().to_lowercase())
            .collect();
        existing
            .tasks
            .extend(incoming.tasks.into_iter().filter(|task| {
                dests.insert(task.dest.to_string_lossy().to_lowercase())
            }));
        existing.concurrency = incoming.concurrency;
        existing.use_mirror = incoming.use_mirror;
    } else {
        *slot = Some(incoming);
    }
}

/// 取出并清空失败批次；重试后再次失败会重新写回
pub fn take_failed_batch() -> Option<FailedBatch> {
    failed_batch_slot().lock().ok().and_then(|mut slot| slot.take())
}

/// 新任务开始前清空上一轮遗留的失败批次。
/// 同一次作业内的多个批次由 store_failed_batch 合并，避免后面的小批次
/// 把前面真正失败的文件从「重试失败文件」里覆盖掉。
pub fn reset_failed_batch() {
    store_failed_batch(None);
}

pub async fn run_tasks(
    app: &AppHandle,
    client: &reqwest::Client,
    tasks: Vec<FileTask>,
    concurrency: usize,
    use_mirror: bool,
    progress: Arc<ProgressState>,
) -> Result<()> {
    let outcome =
        run_tasks_reporting(app, client, tasks, concurrency, use_mirror, progress).await?;
    if let Some((_, message)) = outcome.failed.into_iter().next() {
        return Err(anyhow!("{message}"));
    }
    Ok(())
}

/// 与 run_tasks 相同，但把失败文件收集起来返回，便于分批容错。
pub async fn run_tasks_reporting(
    app: &AppHandle,
    client: &reqwest::Client,
    tasks: Vec<FileTask>,
    concurrency: usize,
    use_mirror: bool,
    progress: Arc<ProgressState>,
) -> Result<DownloadOutcome> {
    // 同一个目标路径只保留一个任务：整合包清单偶尔会把同一文件列两遍，
    // 两个任务同时写同一个 .part 会互相截断，最后内容必然对不上
    let mut seen_dest: std::collections::HashSet<String> = std::collections::HashSet::new();
    let tasks: Vec<FileTask> = tasks
        .into_iter()
        .filter(|task| seen_dest.insert(task.dest.to_string_lossy().to_lowercase()))
        .collect();

    let total_files = tasks.len() as u64;
    let total_bytes: u64 = tasks.iter().map(|t| t.size).sum();
    progress.begin_phase(total_files, total_bytes);
    progress.active.store(true, Ordering::Relaxed);

    if tasks.is_empty() {
        progress.active.store(false, Ordering::Relaxed);
        emit_now(app, &progress);
        return Ok(DownloadOutcome::default());
    }

    start_ticker(app.clone(), progress.clone());
    emit_now(app, &progress);

    let conc = concurrency.clamp(1, MAX_CONCURRENCY);
    let mirror_failures = Arc::new(AtomicU64::new(0));
    let pending = tasks;
    let mut index = 0usize;
    let mut in_flight: FuturesUnordered<_> = FuturesUnordered::new();
    let mut limit = conc;
    let mut failed: Vec<(String, String)> = Vec::new();
    let mut failed_tasks: Vec<FileTask> = Vec::new();
    let mut cancel_err: Option<anyhow::Error> = None;
    let mut recent_ok = 0u32;
    let mut recent_fail = 0u32;

    loop {
        // 按当前并发上限补满在途任务
        while in_flight.len() < limit && index < pending.len() {
            let task = pending[index].clone();
            index += 1;
            let client = client.clone();
            let progress = progress.clone();
            let mirror_failures = mirror_failures.clone();
            in_flight.push(async move {
                let result =
                    download_one(&client, &task, use_mirror, &mirror_failures, &progress).await;
                (task, result)
            });
        }
        if in_flight.is_empty() {
            if index >= pending.len() {
                break;
            }
            continue;
        }
        // 取消不能只靠任务自己返回错误：所有并发请求都卡在网络上时，
        // 主循环永远等不到结果。这里用 select 监听取消标志，一旦取消就
        // 跳出并丢弃在途任务，正在进行的请求随之被打断。
        tokio::select! {
            biased;
            _ = wait_cancel(&progress) => {
                cancel_err = Some(anyhow!("下载已取消"));
                break;
            }
            next = in_flight.next() => {
                match next {
                    Some((_, Ok(()))) => {
                        recent_ok += 1;
                    }
                    Some((task, Err(err))) => {
                        if progress.is_cancelled() {
                            cancel_err = Some(err);
                            break;
                        }
                        failed.push((task.label.clone(), err.to_string()));
                        failed_tasks.push(task);
                        recent_fail += 1;
                    }
                    None => {}
                }
                // 自适应并发：失败率高就减半，稳定成功再慢慢加回上限
                if recent_ok + recent_fail >= 16 {
                    let rate = recent_fail as f64 / (recent_ok + recent_fail) as f64;
                    if rate > 0.2 {
                        limit = (limit / 2).max(4).min(conc);
                    } else if rate < 0.05 {
                        limit = (limit + 4).min(conc);
                    }
                    recent_ok = 0;
                    recent_fail = 0;
                }
            }
        }
    }
    drop(in_flight);

    // 失败项留在全局槽里，供前端的「重试失败文件」只重跑这些任务。
    // 只在确实有失败时写入：空批次不清空已有记录，避免同一作业里
    // 后面的小批次把前一批的失败项覆盖掉
    if !failed_tasks.is_empty() {
        store_failed_batch(Some(FailedBatch {
            tasks: failed_tasks,
            concurrency: conc,
            use_mirror,
        }));
    }

    progress.active.store(false, Ordering::Relaxed);
    emit_now(app, &progress);
    prune_download_cache().await;

    if let Some(err) = cancel_err {
        return Err(err);
    }
    Ok(DownloadOutcome { failed })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// 极简 HTTP/1.1 测试服务：按请求文本返回固定响应，用完即断
    async fn spawn_server(
        handler: Arc<dyn Fn(&str) -> Vec<u8> + Send + Sync>,
    ) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let handler = handler.clone();
                tokio::spawn(async move {
                    let mut buffer = [0u8; 8192];
                    let read = socket.read(&mut buffer).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                    let response = handler(&request);
                    let _ = socket.write_all(&response).await;
                    let _ = socket.flush().await;
                });
            }
        });
        format!("http://{addr}/file")
    }

    fn response(status: &str, length: usize, extra: &str, body: &[u8]) -> Vec<u8> {
        let mut text = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {length}\r\nConnection: close\r\n{extra}\r\n"
        )
        .into_bytes();
        text.extend_from_slice(body);
        text
    }

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("docl-download-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(PathBuf::from(format!("{}.part", path.display())));
        path
    }

    /// 服务端支持 Range：已有 .part 时从断点续传，拼出完整文件
    #[tokio::test]
    async fn resumes_from_existing_part() {
        let body = b"0123456789abcdef".to_vec();
        let full = body.clone();
        let url = spawn_server(Arc::new(move |request: &str| {
            if request.contains("Range: bytes=8-") {
                response(
                    "206 Partial Content",
                    full.len() - 8,
                    "Content-Range: bytes 8-15/16\r\n",
                    &full[8..],
                )
            } else {
                response("200 OK", full.len(), "", &full)
            }
        }))
        .await;

        let dest = temp_path("resume.bin");
        let part = PathBuf::from(format!("{}.part", dest.display()));
        std::fs::write(&part, b"01234567").unwrap();

        let mut progress = |_: u64, _: u64| {};
        download_stream(&reqwest::Client::new(), &[url], &dest, &mut progress)
            .await
            .expect("续传失败");
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert!(!part.exists(), "完成后应清理 .part");
    }

    /// 服务端不认 Range（回 200 整份）：必须丢弃旧 .part 重写，不能拼出脏数据
    #[tokio::test]
    async fn restarts_when_server_ignores_range() {
        let body = b"payload".to_vec();
        let full = body.clone();
        let url =
            spawn_server(Arc::new(move |_| response("200 OK", full.len(), "", &full))).await;
        let dest = temp_path("no-range.bin");
        let part = PathBuf::from(format!("{}.part", dest.display()));
        std::fs::write(&part, b"garbage").unwrap();

        let mut progress = |_: u64, _: u64| {};
        download_stream(&reqwest::Client::new(), &[url], &dest, &mut progress)
            .await
            .expect("整份重下失败");
        assert_eq!(std::fs::read(&dest).unwrap(), body);
    }

    /// 第一个地址 500，应自动换到第二个候选地址
    #[tokio::test]
    async fn falls_back_to_second_url() {
        let body = b"second".to_vec();
        let full = body.clone();
        let bad = spawn_server(Arc::new(|_| response("500 Internal Server Error", 0, "", &[]))).await;
        let good =
            spawn_server(Arc::new(move |_| response("200 OK", full.len(), "", &full))).await;
        let dest = temp_path("fallback.bin");

        let mut progress = |_: u64, _: u64| {};
        download_stream(&reqwest::Client::new(), &[bad, good], &dest, &mut progress)
            .await
            .expect("换源失败");
        assert_eq!(std::fs::read(&dest).unwrap(), body);
    }

    /// 服务端声明 16 字节却提前断开：应报错且不落盘；
    /// 已收到的字节保留在 .part 里，供下次续传
    #[tokio::test]
    async fn rejects_short_body_and_keeps_part() {
        let url = spawn_server(Arc::new(|_| {
            // 声明 16 字节，实际只给 8 字节，然后正常结束
            response("200 OK", 16, "", b"12345678")
        }))
        .await;
        let dest = temp_path("short.bin");
        let part = PathBuf::from(format!("{}.part", dest.display()));

        let mut progress = |_: u64, _: u64| {};
        let result = download_stream(&reqwest::Client::new(), &[url], &dest, &mut progress).await;
        assert!(result.is_err(), "长度不符不应被当成成功");
        assert!(!dest.exists());
        assert_eq!(std::fs::read(&part).map(|data| data.len()).unwrap_or(0), 8);
    }

    /// `.part` 已经完整（上次下完但没改名）：不再发请求，直接校验后落盘
    #[tokio::test]
    async fn finalizes_complete_part_without_network() {
        let body = b"already-complete".to_vec();
        let sha1 = format!("{:x}", Sha1::digest(&body));
        let dest = temp_path("complete.bin");
        let part = PathBuf::from(format!("{}.part", dest.display()));
        std::fs::write(&part, &body).unwrap();

        let task = FileTask {
            // 故意给一个连不上的地址，验证走的是本地完成分支
            url: "http://127.0.0.1:1/nope".into(),
            dest: dest.clone(),
            sha1: Some(sha1),
            size: body.len() as u64,
            label: "complete.bin".into(),
        };
        let progress = ProgressState::new("test");
        fetch_to_file(
            &reqwest::Client::new(),
            &task.url,
            None,
            &task,
            &progress,
            false,
        )
        .await
        .expect("已完成的 part 应直接落盘");
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert_eq!(progress.bytes_done.load(Ordering::Relaxed), body.len() as u64);
    }


    /// 失败重试时要把本次已计入的字节扣回，否则进度会虚高到 100%
    #[test]
    fn rollback_bytes_when_not_committed() {
        let progress = ProgressState::new("test");
        progress.bytes_done.store(1000, Ordering::Relaxed);
        {
            let mut tracker = ByteTracker::new(progress.as_ref());
            tracker.add(200);
            tracker.add(300);
            assert_eq!(progress.bytes_done.load(Ordering::Relaxed), 1500);
            assert_eq!(tracker.written(), 500);
        }
        assert_eq!(progress.bytes_done.load(Ordering::Relaxed), 1000);
    }

    #[test]
    fn keeps_bytes_when_committed() {
        let progress = ProgressState::new("test");
        {
            let mut tracker = ByteTracker::new(progress.as_ref());
            tracker.add(700);
            tracker.commit();
        }
        assert_eq!(progress.bytes_done.load(Ordering::Relaxed), 700);
    }
}
