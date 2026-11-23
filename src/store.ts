import { computed, reactive, ref } from "vue";
import {
  api,
  onGameExit,
  onGameLog,
  onInstallProgress,
  onMsLogin,
  onMsLoginAccount,
  type AccountView,
  type AccountSkin,
  type Config,
  type CustomTheme,
  type DeviceCode,
  type Favorite,
  type CfProject,
  type InstalledVersion,
  type InstanceMeta,
  type JavaInfo,
  type LogLine,
  type PreflightIssue,
  type ProgressSnapshot,
  type SystemMemory,
  type UpdateInfo,
  type VersionSummary,
} from "./api";

export type Page =
  | "home"
  | "versions"
  | "download"
  | "multiplayer"
  | "accounts"
  | "skins"
  | "instance"
  | "settings";

export type JobStatus =
  | "queued"
  | "running"
  | "paused"
  | "done"
  | "partial"
  | "failed"
  | "cancelled";

export interface DownloadJob {
  id: string;
  title: string;
  stage: string;
  status: JobStatus;
  progress: ProgressSnapshot | null;
  error: string | null;
  partial: boolean;
  createdAt: number;
  finishedAt: number | null;
  /// 去重键：同一版本/同一目标文件同时只允许一个任务
  key?: string;
}

export const store = reactive({
  page: "home" as Page,
  instanceId: null as string | null,
  /// 实例内容被外部改动（例如拖入导入）后自增，用于通知已打开的实例页刷新
  contentRevision: 0,

  metas: {} as Record<string, InstanceMeta>,
  communityContext: null as {
    kind: string;
    instanceId: string;
    gameVersion: string;
    loader: string;
    query?: string;
    projectId?: string;
    /// 从收藏页跳进来时直接打开的项目
    project?: CfProject;
    /// 与 kind 不同的实际安装分类，例如数据包需要带上目标世界
    installKind?: string;
  } | null,
  /// 社区资源收藏（本地 JSON），按收藏时间倒序
  favorites: [] as Favorite[],
  config: null as Config | null,
  versions: [] as VersionSummary[],
  installed: [] as InstalledVersion[],
  /// 各游戏目录的版本数量，键为目录绝对路径
  dirCounts: {} as Record<string, number>,
  accounts: [] as AccountView[],
  javaList: [] as JavaInfo[],
  systemMemory: { total_mb: 0, available_mb: 0, recommended_mb: 0 } as SystemMemory,
  loadingVersions: false,
  progress: null as ProgressSnapshot | null,
  installingId: null as string | null,
  /// 下载中心的任务队列（运行 / 排队 / 暂停 / 完成 / 失败）
  jobs: [] as DownloadJob[],
  log: [] as LogLine[],
  /// 运行日志弹窗是否打开
  logPanel: false,
  /// 截图/录屏时遮挡账号 UUID 等标识（仅开发模式生效）
  maskIds: false,
  gameRunning: false,
  runningVersionId: null as string | null,
  stoppedByUser: false,
  crashOpen: false,
  crashVersionId: null as string | null,
  crashExitCode: null as number | null,
  backgroundUrl: "",
  backgroundPreset: "",
  downloadPanel: "hidden" as "hidden" | "open" | "ball",
  lastProgress: null as ProgressSnapshot | null,
  lastJobTitle: "",
  lastJobError: null as string | null,
  /// 任务完成但有文件没下载成功：界面显示「部分完成」而不是「安装失败」
  lastJobPartial: false,
  accountSkin: null as AccountSkin | null,
  defaultSkin: "",
  skinLoading: false,
  skinEditorOpen: false,
  toast: null as { text: string; kind: "ok" | "err" } | null,
  msLogin: null as {
    code: DeviceCode | null;
    status: "loading" | "waiting" | "ok" | "err";
    message?: string;
  } | null,
  /// Java 检查弹窗：missing = 没有可用运行时；mismatch = 手动指定的 Java 与版本需求不符
  javaPrompt: null as null | {
    kind: "missing" | "mismatch";
    message: string;
    required?: number;
    current?: number;
    /// 「仍然启动」时重跑这次启动（跳过预检）
    retry?: () => void;
  },
  /// 启动前自检弹窗
  preflightPrompt: null as null | {
    versionId: string;
    issues: PreflightIssue[];
    blocking: boolean;
    /// 「仍然启动」时重跑这次启动（跳过自检）
    retry: () => void;
  },
  /// 打开设置页时要定位的分类（打开后被设置页消费并清空）
  settingsFocus: null as string | null,
});

export const selectedVersion = computed<InstalledVersion | null>(() => {
  const id = store.config?.selected_version;
  return store.installed.find((v) => v.id === id) ?? store.installed[0] ?? null;
});

export const selectedAccount = computed<AccountView | null>(() => {
  const id = store.config?.selected_account;
  return store.accounts.find((a) => a.id === id) ?? store.accounts[0] ?? null;
});

/// 启动时自动检查到的更新，供设置页直接显示安装入口
export const startupUpdate = ref<{ info: UpdateInfo; path: string | null } | null>(null);

let toastTimer: number | undefined;

export function notify(text: string, kind: "ok" | "err" = "ok") {
  store.toast = { text, kind };
  if (toastTimer) window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => {
    store.toast = null;
  }, 3200);
}

export async function refreshConfig() {
  store.config = await api.getConfig();
}

export async function refreshInstalled() {
  store.installed = await api.installedVersions();
}

export async function refreshInstanceMetas() {
  try {
    store.metas = await api.instanceMetas();
  } catch {
    store.metas = {};
  }
}

export async function refreshDirCounts() {
  try {
    store.dirCounts = await api.gameDirVersionCounts();
  } catch {
    store.dirCounts = {};
  }
}

function normalizeDir(path: string) {
  return path.replace(/\//g, "\\").replace(/\\+$/, "").toLowerCase();
}

export function sameDir(a: string, b: string) {
  return normalizeDir(a) === normalizeDir(b);
}

/// 切换活跃游戏目录；当前版本不在新目录时自动选中该目录第一个版本，
/// 避免首页和状态栏出现「无当前版本」。
export async function switchGameDir(path: string) {
  const dirs = store.config?.game_dirs ?? [];
  const target = dirs.find((entry) => sameDir(entry.path, path));
  const label = target?.name ?? path;
  if (store.config && sameDir(store.config.game_dir, path)) return;
  try {
    store.config = await api.switchGameDir(path);
    store.instanceId = null;
    await Promise.all([refreshInstalled(), refreshInstanceMetas(), refreshDirCounts()]);
    const current = store.config?.selected_version;
    if (!store.installed.some((item) => item.id === current)) {
      await saveConfig({ selected_version: store.installed[0]?.id ?? null });
    }
    notify(`已切换到 ${label}`);
  } catch (err) {
    notify(`切换目录失败：${String(err)}`, "err");
  }
}

export function openInstance(id: string) {
  store.instanceId = id;
  store.page = "instance";
}

/// 打开崩溃分析弹层；不传版本时优先用正在运行或当前选中的版本。
export function openCrashDialog(versionId?: string | null, exitCode: number | null = null) {
  const target = versionId ?? store.runningVersionId ?? selectedVersion.value?.id ?? null;
  if (!target) {
    notify("请先选择一个游戏版本", "err");
    return;
  }
  store.crashVersionId = target;
  store.crashExitCode = exitCode;
  store.crashOpen = true;
}

/// 从版本设置跳到下载页，并带上当前实例的版本与加载器。
export function openCommunity(
  kind: string,
  instanceId: string,
  gameVersion: string,
  loader: string,
  installKind?: string,
  query?: string
) {
  store.communityContext = { kind, instanceId, gameVersion, loader, installKind, query };
  store.page = "download";
}

export async function refreshAccounts() {
  store.accounts = await api.getAccounts();
  await loadSelectedSkin();
}

export async function loadSelectedSkin() {
  const account = selectedAccount.value;
  if (!account) {
    store.accountSkin = null;
    return;
  }
  store.skinLoading = true;
  try {
    store.accountSkin = await api.getAccountSkin(account.id);
  } catch {
    store.accountSkin = null;
  } finally {
    store.skinLoading = false;
  }
}

export async function refreshJava() {
  store.javaList = await api.detectJava();
}

export async function refreshVersions(force = false) {
  if (store.loadingVersions) return;
  store.loadingVersions = true;
  try {
    store.versions = await api.fetchVersions(force);
  } catch (err) {
    notify(`获取版本列表失败：${String(err)}`, "err");
  } finally {
    store.loadingVersions = false;
  }
}

export async function saveConfig(patch: Partial<Config>) {
  if (!store.config) return;
  const next = { ...store.config, ...patch };
  try {
    store.config = await api.saveConfig(next);
    applyTheme(store.config.theme);
    await refreshInstalled();
    await refreshJava();
  } catch (err) {
    notify(`保存设置失败：${String(err)}`, "err");
  }
}

/// 收藏列表：读取失败时保留内存里的旧值，不打扰资源浏览
export async function refreshFavorites() {
  try {
    store.favorites = await api.favoritesList();
  } catch {
    // 收藏文件损坏或不可读时按空处理
  }
}

export function isFavorite(project: { id: string; source: string }) {
  return store.favorites.some(
    (item) => item.id === project.id && item.source === project.source
  );
}

/// 切换收藏状态：kind 为资源分类（mods / modpacks 等），返回切换后是否已收藏
export async function toggleFavorite(
  kind: string,
  project: Omit<Favorite, "kind" | "added_at">
) {
  try {
    const added = await api.favoritesToggle({ ...project, kind, added_at: 0 });
    await refreshFavorites();
    notify(added ? "已加入收藏" : "已取消收藏");
    return added;
  } catch (err) {
    notify(`收藏失败：${String(err)}`, "err");
    return false;
  }
}

/// 自定义主题会写这些内联变量；切回内置主题时要全部清掉
const CUSTOM_THEME_VARS = [
  "--paper",
  "--paper-2",
  "--panel",
  "--panel-2",
  "--walnut",
  "--walnut-deep",
  "--walnut-light",
  "--moss",
  "--moss-bright",
  "--moss-soft",
  "--amber",
  "--danger",
  "--danger-soft",
  "--ink",
  "--ink-soft",
  "--muted",
  "--line",
  "--line-strong",
  "--hover",
  "--glass-rgb",
  "--bar-a",
  "--bar-b",
  "--hero-a",
  "--hero-b",
  "--hero-c",
  "--btn-a",
  "--btn-b",
  "--btn-c",
  "--btn-border",
  "--btn-fg",
  "--accent-rgb",
  "--ambient-a",
  "--ambient-b",
];

function isHexColor(value: string) {
  return /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.test(value);
}

/// 解析 #rgb / #rrggbb，供派生 rgba 用
function hexToRgb(value: string): [number, number, number] | null {
  const clean = value.replace("#", "").trim();
  const full =
    clean.length === 3
      ? clean
          .split("")
          .map((ch) => ch + ch)
          .join("")
      : clean;
  if (full.length !== 6 || /[^0-9a-f]/i.test(full)) return null;
  const num = Number.parseInt(full, 16);
  return [(num >> 16) & 255, (num >> 8) & 255, num & 255];
}

function clearCustomTheme() {
  const root = document.documentElement;
  for (const name of CUSTOM_THEME_VARS) root.style.removeProperty(name);
  root.style.removeProperty("color-scheme");
}

/// 按一个自定义主题的关键色派生整套变量，写成内联样式盖住内置主题
export function applyCustomTheme(theme: CustomTheme) {
  const root = document.documentElement;
  const pick = (value: string, fallback: string) =>
    isHexColor(value) ? value : fallback;
  const paper = pick(theme.paper, "#f4ebdd");
  const panel = pick(theme.panel, "#fffdf6");
  const primary = pick(theme.primary, "#6f4224");
  const accent = pick(theme.accent, "#5e7b49");
  const title = pick(theme.title, "#462812");
  const text = pick(theme.text, "#2b2118");
  const muted = pick(theme.muted, "#96836d");
  const line = pick(theme.line, "#e4d5bb");

  const set = (name: string, value: string) => root.style.setProperty(name, value);
  set("--paper", paper);
  set("--paper-2", `color-mix(in srgb, ${paper} 93%, ${text})`);
  set("--panel", panel);
  set("--panel-2", `color-mix(in srgb, ${panel} 92%, ${text})`);
  set("--walnut", primary);
  set("--walnut-deep", title);
  set("--walnut-light", `color-mix(in srgb, ${primary} 55%, #fff)`);
  set("--moss", accent);
  set("--moss-bright", `color-mix(in srgb, ${accent} 62%, #fff)`);
  set("--moss-soft", `color-mix(in srgb, ${accent} 16%, ${paper})`);
  set("--amber", `color-mix(in srgb, ${accent} 30%, #d9a05b)`);
  set("--danger", `color-mix(in srgb, #a34e36 78%, ${text})`);
  set("--danger-soft", `color-mix(in srgb, #a34e36 18%, ${panel})`);
  set("--ink", text);
  set("--ink-soft", `color-mix(in srgb, ${text} 74%, ${paper})`);
  set("--muted", muted);
  set("--line", line);
  set("--line-strong", `color-mix(in srgb, ${line} 58%, ${text})`);
  set("--hover", `color-mix(in srgb, ${paper} 88%, ${accent})`);

  const panelRgb = hexToRgb(panel) ?? [255, 253, 246];
  const primaryRgb = hexToRgb(primary) ?? [111, 66, 36];
  const accentRgb = hexToRgb(accent) ?? [94, 123, 73];
  const amberRgb = hexToRgb("#d9a05b") ?? [217, 160, 91];
  const paperRgb = hexToRgb(paper) ?? [244, 235, 221];
  // 深色底的自定义主题让原生控件 / 滚动条也切到深色
  const paperLum =
    0.2126 * paperRgb[0] + 0.7152 * paperRgb[1] + 0.0722 * paperRgb[2];
  root.style.colorScheme = paperLum < 120 ? "dark" : "light";
  set("--glass-rgb", `${panelRgb[0]}, ${panelRgb[1]}, ${panelRgb[2]}`);
  set("--accent-rgb", `${accentRgb[0]}, ${accentRgb[1]}, ${accentRgb[2]}`);
  set("--ambient-a", `${amberRgb[0]}, ${amberRgb[1]}, ${amberRgb[2]}`);
  set("--ambient-b", `${accentRgb[0]}, ${accentRgb[1]}, ${accentRgb[2]}`);

  set("--bar-a", `color-mix(in srgb, ${panel} 96%, ${accent})`);
  set("--bar-b", `color-mix(in srgb, ${panel} 90%, ${accent})`);
  set("--hero-a", panel);
  set("--hero-b", `color-mix(in srgb, ${panel} 93%, ${accent})`);
  set("--hero-c", `color-mix(in srgb, ${panel} 86%, ${accent})`);

  set("--btn-a", `color-mix(in srgb, ${primary} 78%, #fff)`);
  set("--btn-b", primary);
  set("--btn-c", `color-mix(in srgb, ${primary} 74%, #000)`);
  set("--btn-border", `color-mix(in srgb, ${primary} 58%, #000)`);
  // 浅色主色上按钮要用深色字
  const light =
    0.2126 * primaryRgb[0] + 0.7152 * primaryRgb[1] + 0.0722 * primaryRgb[2] > 150;
  set("--btn-fg", light ? "#2b2118" : "#fff8ea");
}

export function applyTheme(theme?: string | null, customThemes?: CustomTheme[]) {
  const root = document.documentElement;
  const value = theme || "paper";
  const list = customThemes ?? store.config?.custom_themes ?? [];
  const id = value.startsWith("custom:") ? value.slice(7) : "";
  const custom = id ? list.find((item) => item.id === id) : undefined;
  if (custom) {
    // 先用纸木主题兜底，自定义主题没覆盖到的变量也能拿到合理值
    root.dataset.theme = "paper";
    applyCustomTheme(custom);
    return;
  }
  root.dataset.theme = id ? "paper" : value;
  clearCustomTheme();
}

export async function loadBackground() {
  // 文件夹模式优先：每次启动随机挑一张（PCL 式）
  const dir = store.config?.background_dir;
  if (dir) {
    store.backgroundPreset = "";
    try {
      const picked = await api.pickBackgroundRandom(dir);
      store.backgroundUrl = picked ? await api.backgroundDataUrl(picked) : "";
    } catch {
      store.backgroundUrl = "";
    }
    return;
  }
  const path = store.config?.background_image;
  if (!path) {
    store.backgroundUrl = "";
    store.backgroundPreset = "";
    return;
  }
  if (path.startsWith("builtin:")) {
    store.backgroundPreset = path.slice("builtin:".length);
    store.backgroundUrl = "";
    return;
  }
  store.backgroundPreset = "";
  try {
    store.backgroundUrl = await api.backgroundDataUrl(path);
  } catch {
    store.backgroundUrl = "";
  }
}

/// 背景文件夹模式下「换一张」：重新随机挑选一张
export async function shuffleBackground() {
  const dir = store.config?.background_dir;
  if (!dir) return;
  try {
    const picked = await api.pickBackgroundRandom(dir);
    store.backgroundUrl = picked ? await api.backgroundDataUrl(picked) : "";
    notify("已换一张背景");
  } catch (err) {
    notify(`换背景失败：${String(err)}`, "err");
  }
}

/// 同时运行的下载任务上限；其余任务在队列里等待
export const MAX_PARALLEL_JOBS = 2;

interface JobSpec {
  title: string;
  stage: string;
  message: string;
  doneMessage?: string;
  errorPrefix?: string;
  /// 去重键：同键的任务还在队列/运行时不重复入队
  key?: string;
  run: (jobId: string, job: DownloadJob) => Promise<unknown>;
}

const jobSpecs = new Map<string, JobSpec>();
const jobResolvers = new Map<string, (value: unknown) => void>();
const jobCancelFlags = new Map<string, boolean>();
let jobSeq = 0;

function primeSnapshot(stage: string, message: string, job: string): ProgressSnapshot {
  return {
    job,
    stage,
    message,
    steps: [{ label: message, done: false, percent: 0 }],
    files_done: 0,
    files_total: 0,
    bytes_done: 0,
    bytes_total: 0,
    elapsed_ms: 0,
    active: true,
    paused: false,
  };
}

/// 真正在下载的任务；暂停的任务不占并发槽位，让排队任务先跑
function runningJobs() {
  return store.jobs.filter((job) => job.status === "running");
}

function pausedJobs() {
  return store.jobs.filter((job) => job.status === "paused");
}

function syncInstalling() {
  store.installingId = (runningJobs()[0] ?? pausedJobs()[0])?.title ?? null;
}

export function jobById(id: string) {
  return store.jobs.find((job) => job.id === id) ?? null;
}

/// 读取任务当前状态；包一层函数绕过 TS 对属性赋值的字面量收窄
function currentStatus(job: DownloadJob): JobStatus {
  return job.status;
}

/// 排空队列：有空位就启动下一个
export function scheduleJobs() {
  let free = MAX_PARALLEL_JOBS - runningJobs().length;
  for (const job of store.jobs) {
    if (free <= 0) break;
    if (job.status !== "queued") continue;
    free -= 1;
    void startJob(job);
  }
  syncInstalling();
}

async function startJob(job: DownloadJob) {
  const spec = jobSpecs.get(job.id);
  if (!spec || job.status !== "queued") return;
  job.status = "running";
  job.error = null;
  job.partial = false;
  job.finishedAt = null;
  job.progress = primeSnapshot(job.stage, spec.message, job.id);
  syncInstalling();
  try {
    const result = await spec.run(job.id, job);
    // 任务运行期间状态可能被取消按钮改写，这里用函数取一次最新值避免类型收窄
    const after = currentStatus(job);
    if (after !== "cancelled") {
      job.status = job.partial ? "partial" : "done";
    }
    if (spec.doneMessage && currentStatus(job) !== "cancelled") notify(spec.doneMessage);
    if (currentStatus(job) === "done") jobSpecs.delete(job.id);
    jobResolvers.get(job.id)?.(result);
  } catch (err) {
    const cancelled = jobCancelFlags.get(job.id) || currentStatus(job) === "cancelled";
    job.status = cancelled ? "cancelled" : "failed";
    job.error = cancelled ? "已取消" : String(err);
    if (!cancelled) notify(`${spec.errorPrefix ?? "任务失败"}：${job.error}`, "err");
    jobResolvers.get(job.id)?.(undefined);
  } finally {
    job.finishedAt = Date.now();
    jobResolvers.delete(job.id);
    jobCancelFlags.delete(job.id);
    syncInstalling();
    scheduleJobs();
  }
}

/// 把一次下载/安装登记成任务：有空位立即开始，否则排队
export function enqueueJob<T>(
  spec: Omit<JobSpec, "run"> & { run: (jobId: string, job: DownloadJob) => Promise<T> }
): Promise<T | undefined> {
  // 同一版本/同一文件重复入队会并发写同一个 .part，直接挡掉
  if (spec.key) {
    const existing = store.jobs.find(
      (job) =>
        job.key === spec.key &&
        (job.status === "queued" || job.status === "running" || job.status === "paused")
    );
    if (existing) {
      notify(`「${existing.title}」已在任务队列中`, "err");
      return Promise.resolve(undefined);
    }
  }
  const id = `job-${Date.now().toString(36)}-${++jobSeq}`;
  const job: DownloadJob = {
    id,
    title: spec.title,
    stage: spec.stage,
    status: "queued",
    progress: primeSnapshot(spec.stage, spec.message, id),
    error: null,
    partial: false,
    createdAt: Date.now(),
    finishedAt: null,
    key: spec.key,
  };
  store.jobs.push(job);
  jobSpecs.set(id, spec as JobSpec);
  jobCancelFlags.set(id, false);
  if (store.downloadPanel === "hidden") store.downloadPanel = "open";
  return new Promise<T | undefined>((resolve) => {
    jobResolvers.set(id, resolve as (value: unknown) => void);
    scheduleJobs();
  });
}

export async function pauseJob(id: string) {
  const job = jobById(id);
  if (!job || job.status !== "running") return;
  try {
    await api.pauseJob(id);
    job.status = "paused";
    // 让出并发槽位，排队任务可以立刻开始
    scheduleJobs();
  } catch (err) {
    notify(`暂停失败：${String(err)}`, "err");
  }
}

/// 批量暂停/继续：下载页任务面板用
export async function pauseAllJobs() {
  for (const job of runningJobs()) {
    if (job.status === "running") await pauseJob(job.id);
  }
}

export async function resumeAllJobs() {
  for (const job of [...store.jobs]) {
    if (job.status === "paused") await resumeJob(job.id);
  }
}

export async function resumeJob(id: string) {
  const job = jobById(id);
  if (!job || job.status !== "paused") return;
  // 暂停不占槽位，恢复时若并行数已满，只能继续等待
  if (runningJobs().length >= MAX_PARALLEL_JOBS) {
    notify(`已有 ${MAX_PARALLEL_JOBS} 个任务在下载，请先暂停或取消一个`, "err");
    return;
  }
  try {
    await api.resumeJob(id);
    job.status = "running";
  } catch (err) {
    notify(`继续失败：${String(err)}`, "err");
  }
}

export async function cancelJob(id: string) {
  const job = jobById(id);
  if (!job) return;
  if (job.status === "queued") {
    job.status = "cancelled";
    job.error = "已取消";
    job.finishedAt = Date.now();
    jobResolvers.get(id)?.(undefined);
    jobResolvers.delete(id);
    scheduleJobs();
    return;
  }
  jobCancelFlags.set(id, true);
  job.status = "cancelled";
  try {
    await api.cancelInstall(id);
  } catch {
    // 任务可能已经结束，忽略
  }
}

export function retryJob(id: string) {
  const spec = jobSpecs.get(id);
  if (!spec) {
    notify("该任务无法重试", "err");
    return;
  }
  dismissJob(id);
  void enqueueJob(spec);
}

export function dismissJob(id: string) {
  const job = jobById(id);
  if (!job) return;
  if (job.status === "running" || job.status === "paused") {
    void cancelJob(id);
  }
  jobSpecs.delete(id);
  jobResolvers.delete(id);
  jobCancelFlags.delete(id);
  store.jobs = store.jobs.filter((item) => item.id !== id);
  syncInstalling();
}

export function clearFinishedJobs() {
  for (const job of [...store.jobs]) {
    if (job.status !== "running" && job.status !== "paused" && job.status !== "queued") {
      dismissJob(job.id);
    }
  }
}


export function installVersion(id: string) {
  return enqueueJob({
    title: id,
    stage: "install",
    message: `准备安装 ${id}…`,
    doneMessage: `${id} 安装完成`,
    errorPrefix: "安装失败",
    key: `version:${id}`,
    run: async (jobId) => {
      await api.installVersion(id, jobId);
      await refreshInstalled();
      await refreshConfig();
    },
  });
}

export function installLoader(
  versionId: string,
  loader: string,
  loaderVersion?: string
) {
  return enqueueJob({
    title: `${versionId} · ${loader}${loaderVersion ? ` ${loaderVersion}` : ""}`,
    stage: "install",
    message: `准备安装 ${loader}…`,
    doneMessage: `${loader} 安装完成`,
    errorPrefix: "安装失败",
    key: `loader:${versionId}:${loader}`,
    run: async (jobId) => {
      await api.installLoader(versionId, loader, loaderVersion, jobId);
      await refreshInstalled();
      await refreshConfig();
    },
  });
}

/// 重装加载器：清理损坏的版本目录后按原加载器版本重装，实例文件不受影响
export async function reinstallLoader(versionId: string) {
  const result = await enqueueJob<boolean>({
    title: "重装加载器",
    stage: "install",
    message: "准备重装加载器…",
    doneMessage: "加载器已重装",
    errorPrefix: "重装加载器失败",
    key: `reload:${versionId}`,
    run: async (jobId) => {
      await api.reinstallLoader(versionId, jobId);
      await refreshInstalled();
      await refreshConfig();
      return true;
    },
  });
  return result === true;
}

export function installCfFile(
  versionId: string,
  kind: string,
  url: string,
  fileName: string,
  title: string
) {
  return enqueueJob({
    title,
    stage: "install",
    message: `准备下载 ${title}…`,
    errorPrefix: "下载失败",
    key: `content:${versionId}:${url}`,
    run: async (jobId, job) => {
      const result = await api.cfInstallFile(versionId, kind, url, fileName, jobId);
      const deps = result.dependencies;
      const notes: string[] = [];
      if (deps.installed.length) notes.push(`已自动补齐前置 ${deps.installed.join("、")}`);
      if (deps.failed.length) notes.push(`前置下载失败：${deps.failed.join("、")}`);
      notify(`${title} 下载完成${notes.length ? `，${notes.join("；")}` : ""}`);
      if (deps.unresolved.length) {
        job.partial = true;
        job.error = `缺少前置：${deps.unresolved.join("、")}。当前实例的版本或加载器下没有找到对应文件，请手动安装。`;
      }
    },
  });
}

export function installWorld(
  versionId: string,
  url: string,
  fileName: string,
  title: string
) {
  return enqueueJob({
    title: `地图 · ${title}`,
    stage: "install",
    message: `准备下载地图 ${title}…`,
    errorPrefix: "地图安装失败",
    key: `world:${versionId}:${url}`,
    run: async (jobId) => {
      const path = await api.cfInstallWorld(versionId, url, fileName, jobId);
      const folder = path.split(/[\\/]/).pop() || "saves";
      notify(`地图已安装到存档：${folder}`);
    },
  });
}

export async function downloadModpack(
  url: string,
  fileName: string,
  title: string
): Promise<string | null> {
  const result = await enqueueJob<string>({
    title: `整合包 · ${fileName}`,
    stage: "install",
    message: `准备下载 ${title}…`,
    doneMessage: `${title} 下载完成`,
    errorPrefix: "下载失败",
    key: `pack:${url}`,
    run: (jobId) => api.modpackDownload(url, fileName, jobId),
  });
  return result ?? null;
}

export async function importModpack(
  path: string,
  options: import("./api").ModpackImportOptions,
  title: string
) {
  return (
    (await enqueueJob({
      title: `导入整合包 · ${title}`,
      stage: "install",
      message: `准备导入 ${title}…`,
      errorPrefix: "导入失败",
      key: `pack-import:${path}`,
      run: async (jobId, job) => {
        const result = await api.modpackImport(path, options, jobId);
        await refreshInstalled();
        await refreshConfig();
        await refreshInstanceMetas();
        notify(
          result.failed.length
            ? `已导入 ${result.display_name}，${result.failed.length} 个文件未能下载`
            : `已导入 ${result.display_name}`
        );
        if (result.failed.length) {
          const shown = result.failed.slice(0, 5).join("；");
          job.partial = true;
          job.error =
            result.failed.length > 5 ? `${shown} 等 ${result.failed.length} 个` : shown;
        }
        return result;
      },
    })) ?? null
  );
}

/// 只重试某个任务上一批失败的文件，不必重新导入整个整合包
export function retryFailedDownloads(sourceJob?: string) {
  return enqueueJob({
    title: "重试失败文件",
    stage: "repair",
    message: "准备重试失败文件…",
    errorPrefix: "重试失败",
    key: `retry:${sourceJob ?? "latest"}`,
    run: async (jobId, job) => {
      const result = await api.retryFailedDownloads(sourceJob, jobId);
      if (result.failed.length) {
        job.partial = true;
        job.error =
          `仍有 ${result.failed.length} 个文件未能下载：` +
          result.failed.slice(0, 5).join("；");
        notify(`仍有 ${result.failed.length} 个文件未能下载`, "err");
      } else {
        notify("失败文件已全部补齐");
      }
    },
  });
}

/// 一键自愈：修清单、补缺失文件、禁用重复模组
export async function repairInstance(
  versionId: string,
  options?: import("./api").RepairOptions
) {
  return (
    (await enqueueJob({
      title: "一键修复",
      stage: "repair",
      message: "准备检查版本文件…",
      errorPrefix: "修复失败",
      key: `repair:${versionId}`,
      run: (jobId) => api.repairInstance(versionId, options, jobId),
    })) ?? null
  );
}

/// 给实例装一个推荐的光影加载器（Iris / Oculus）
export async function installShaderLoader(versionId: string) {
  const result = await enqueueJob<string>({
    title: `光影加载器 · ${versionId}`,
    stage: "shader",
    message: "正在安装光影加载器…",
    errorPrefix: "安装光影加载器失败",
    key: `shader:${versionId}`,
    run: async (jobId) => {
      const name = await api.installShaderLoader(versionId, jobId);
      notify(`已安装 ${name}，进游戏就能加载光影了`);
      return name;
    },
  });
  return result ?? null;
}

/// 还原最近一次一键修复前的备份
export async function restoreRepairBackup(versionId: string) {
  try {
    const count = await api.repairRestore(versionId);
    notify(`已还原 ${count} 个文件`);
    return count;
  } catch (err) {
    notify(`还原失败：${String(err)}`, "err");
    return 0;
  }
}

export async function exportModpack(
  versionId: string,
  options: import("./api").ExportOptions,
  title: string
) {
  return (
    (await enqueueJob({
      title: `导出整合包 · ${title}`,
      stage: "install",
      message: "准备导出整合包…",
      errorPrefix: "导出失败",
      key: `export:${versionId}:${options.target}`,
      run: async (jobId) => {
        const result = await api.modpackExport(versionId, options, jobId);
        notify(`已导出到 ${result.paths[0] ?? options.target}`);
        return result;
      },
    })) ?? null
  );
}

/// 用新版整合包更新实例：走任务队列，才能显示进度、暂停与取消
export async function updateModpack(
  versionId: string,
  path: string,
  installOptional = false
) {
  return (
    (await enqueueJob({
      title: "更新整合包",
      stage: "modpack-update",
      message: "准备更新整合包…",
      errorPrefix: "更新整合包失败",
      key: `pack-update:${versionId}:${path}`,
      run: async (jobId) =>
        api.modpackUpdateApply(versionId, path, installOptional, jobId),
    })) ?? null
  );
}

export async function installJava(major: number, source = "mojang") {
  const result = await enqueueJob<import("./api").JavaInfo>({
    title: `Java ${major}`,
    stage: "java",
    message: `准备安装 Java ${major}…`,
    doneMessage: `Java ${major} 已安装`,
    errorPrefix: "安装 Java 失败",
    key: `java:${major}:${source}`,
    run: async (jobId) => {
      const info = await api.installJava(major, source, jobId);
      await refreshJava();
      await refreshConfig();
      return info;
    },
  });
  return result ?? null;
}

export async function cancelInstall(jobId?: string) {
  if (jobId) {
    await cancelJob(jobId);
    return;
  }
  const running = runningJobs();
  if (!running.length) {
    await api.cancelInstall();
    return;
  }
  for (const job of running) await cancelJob(job.id);
}

export async function selectVersion(id: string) {
  await saveConfig({ selected_version: id });
}

export async function selectAccount(id: string) {
  await api.selectAccount(id);
  await refreshConfig();
  await loadSelectedSkin();
}

export async function createOffline(name: string) {
  try {
    await api.createOfflineAccount(name);
    await refreshAccounts();
    await refreshConfig();
    notify(`已添加离线账号 ${name}`);
    return true;
  } catch (err) {
    notify(String(err), "err");
    return false;
  }
}

export async function removeAccount(id: string) {
  try {
    await api.removeAccount(id);
    await refreshAccounts();
    await refreshConfig();
    notify("账号已移除");
  } catch (err) {
    notify(String(err), "err");
  }
}

export async function beginMsLogin() {
  store.msLogin = { code: null, status: "loading" };
  try {
    const code = await api.msLoginBegin();
    store.msLogin = { code, status: "waiting" };
  } catch (err) {
    store.msLogin = { code: null, status: "err", message: String(err) };
  }
}

/// 单独暴露停止入口，启动按钮不再承担结束动作。
export async function stopGame() {
  if (!store.gameRunning) return;
  store.stoppedByUser = true;
  try {
    await api.killGame();
    notify("正在停止游戏");
  } catch (err) {
    notify(String(err), "err");
  }
}

/// 启动前的 Java 预检：手动指定的 Java 与版本需求不符时返回详情。
/// 兼容规则与后端 find_java 保持一致：Java 8 时代的版本只认 8，更高版本允许向上兼容；
/// 不符时后端会跳过手动路径改用匹配的运行时。
async function javaMismatchFor(target: string) {
  const required = store.installed.find((v) => v.id === target)?.java_major;
  const javaPath = store.config?.java_path?.trim();
  if (!required || !javaPath) return null;
  if (!store.javaList.length) await refreshJava();
  const manual = store.javaList.find((j) => j.path === javaPath);
  if (!manual) return null;
  if (manual.major >= required && (required > 8 || manual.major === required)) return null;
  return { required, current: manual.major };
}

/// 版本不匹配时给用户看的说明：后端会自动换用匹配的运行时
function javaMismatchMessage(target: string, required: number, current: number) {
  return `「${target}」需要 Java ${required}，当前手动指定的是 Java ${current}，不适用。启动器会自动改用本机匹配的运行时；如果本机没有，启动会失败。`;
}

/// 本会话内已经确认过「只有警告」的实例，避免每次启动都弹同一个框
const preflightConfirmed = new Set<string>();

/// 启动前自检。返回 false 表示发现问题、已弹出提示，本次不应继续启动。
async function preflightGate(target: string, retry: () => void): Promise<boolean> {
  let issues: PreflightIssue[];
  try {
    issues = (await api.preflightCheck(target)).issues;
  } catch {
    // 自检本身出错时放行，不能因为它挡住正常启动
    return true;
  }
  if (!issues.length) return true;
  const blocking = issues.some((issue) => issue.blocking);
  if (!blocking && preflightConfirmed.has(target)) return true;
  store.preflightPrompt = {
    versionId: target,
    issues,
    blocking,
    retry: () => {
      if (!blocking) preflightConfirmed.add(target);
      retry();
    },
  };
  return false;
}

/// Java 预检是异步的，期间挡住重复点击，避免同时拉起两个进程
let launchGuard = false;

export async function launchVersion(
  id?: string,
  opts?: { skipJavaCheck?: boolean; skipPreflight?: boolean }
) {
  const target = id ?? selectedVersion.value?.id;
  if (!target) {
    notify("请先安装一个游戏版本", "err");
    store.page = "download";
    return;
  }
  if (store.gameRunning) {
    // 同一个按钮不能既启动又结束，避免误触把正在启动的游戏杀掉
    notify("游戏已在运行，停止请点「停止游戏」");
    return;
  }
  if (launchGuard) return;
  if (!opts?.skipJavaCheck) {
    let mismatch: Awaited<ReturnType<typeof javaMismatchFor>> = null;
    launchGuard = true;
    try {
      mismatch = await javaMismatchFor(target);
    } finally {
      launchGuard = false;
    }
    if (mismatch) {
      store.javaPrompt = {
        kind: "mismatch",
        message: javaMismatchMessage(target, mismatch.required, mismatch.current),
        ...mismatch,
        retry: () => void launchVersion(target, { skipJavaCheck: true }),
      };
      return;
    }
  }
  if (!opts?.skipPreflight) {
    launchGuard = true;
    let passed = true;
    try {
      passed = await preflightGate(
        target,
        () => void launchVersion(target, { skipJavaCheck: true, skipPreflight: true })
      );
    } finally {
      launchGuard = false;
    }
    if (!passed) return;
  }
  store.runningVersionId = target;
  store.stoppedByUser = false;
  store.gameRunning = true;
  try {
    await api.launchVersion(target);
    notify(`正在启动 ${target}`);
  } catch (err) {
    store.gameRunning = false;
    store.runningVersionId = null;
    const message = String(err);
    if (message.includes("LAUNCH_NO_JAVA")) {
      store.javaPrompt = {
        kind: "missing",
        message: message.replace("LAUNCH_NO_JAVA:", ""),
      };
    } else {
      notify(`启动失败：${message}`, "err");
    }
  }
}

export async function launchServer(
  address: string,
  id?: string,
  opts?: { skipJavaCheck?: boolean; skipPreflight?: boolean }
) {
  const target = id ?? selectedVersion.value?.id;
  if (!target) {
    notify("请先安装一个游戏版本", "err");
    store.page = "download";
    return;
  }
  if (store.gameRunning) {
    notify("游戏已在运行，请先停止", "err");
    return;
  }
  if (launchGuard) return;
  if (!opts?.skipJavaCheck) {
    let mismatch: Awaited<ReturnType<typeof javaMismatchFor>> = null;
    launchGuard = true;
    try {
      mismatch = await javaMismatchFor(target);
    } finally {
      launchGuard = false;
    }
    if (mismatch) {
      store.javaPrompt = {
        kind: "mismatch",
        message: javaMismatchMessage(target, mismatch.required, mismatch.current),
        ...mismatch,
        retry: () => void launchServer(address, target, { skipJavaCheck: true }),
      };
      return;
    }
  }
  if (!opts?.skipPreflight) {
    launchGuard = true;
    let passed = true;
    try {
      passed = await preflightGate(
        target,
        () => void launchServer(address, target, { skipJavaCheck: true, skipPreflight: true })
      );
    } finally {
      launchGuard = false;
    }
    if (!passed) return;
  }
  store.runningVersionId = target;
  store.stoppedByUser = false;
  store.gameRunning = true;
  try {
    await api.launchServer(target, address);
    notify(`正在加入 ${address}`);
  } catch (err) {
    store.gameRunning = false;
    store.runningVersionId = null;
    const message = String(err);
    if (message.includes("LAUNCH_NO_JAVA")) {
      store.javaPrompt = {
        kind: "missing",
        message: message.replace("LAUNCH_NO_JAVA:", ""),
      };
    } else {
      notify(`连接失败：${message}`, "err");
    }
  }
}

export async function rememberServer(address: string) {
  try {
    store.config = await api.rememberServer(address);
  } catch (err) {
    notify(`保存服务器失败：${String(err)}`, "err");
  }
}

export async function forgetServer(address: string) {
  try {
    store.config = await api.forgetServer(address);
    notify("已从最近服务器移除");
  } catch (err) {
    notify(`移除失败：${String(err)}`, "err");
  }
}

export async function uninstallVersion(id: string) {
  try {
    const removed = await api.deleteVersion(id);
    await refreshInstalled();
    if (store.config?.selected_version && removed.includes(store.config.selected_version)) {
      await saveConfig({ selected_version: store.installed[0]?.id ?? null });
    }
    await refreshInstanceMetas();
    notify(
      removed.length > 1
        ? `已删除 ${id} 及 ${removed.length - 1} 个依赖版本`
        : `已删除 ${id}`
    );
  } catch (err) {
    notify(`删除失败：${String(err)}`, "err");
  }
}

export async function setInstanceIcon(id: string, icon: string) {
  try {
    await api.instanceSetIcon(id, icon);
    await refreshInstanceMetas();
  } catch (err) {
    notify(`图标保存失败：${String(err)}`, "err");
  }
}

export async function setInstanceIconImage(id: string, source: string) {
  try {
    await api.instanceSetIconImage(id, source);
    await refreshInstanceMetas();
    notify("已更新版本图标");
  } catch (err) {
    notify(`图标保存失败：${String(err)}`, "err");
  }
}

export async function loadInstanceIconImage(id: string) {
  try {
    return await api.instanceIconImage(id);
  } catch {
    return null;
  }
}

let initialized = false;

export async function initApp() {
  if (initialized) return;
  initialized = true;

  try {
    store.config = await api.getConfig();
    applyTheme(store.config.theme);
    await loadBackground();
  } catch (err) {
    notify(`读取配置失败：${String(err)}`, "err");
  }

  await Promise.allSettled([
    refreshInstalled(),
    refreshAccounts(),
    refreshJava(),
    refreshInstanceMetas(),
    refreshDirCounts(),
    refreshFavorites(),
    api.systemMemory().then((value) => {
      store.systemMemory = value;
    }),
    api.defaultSkin().then((value) => {
      store.defaultSkin = value;
    }),
  ]);

  await onInstallProgress((p) => {
    store.lastProgress = p;
    const job = jobById(p.job);
    if (!job) {
      // 非队列任务（例如陶瓦联机）仍记录最近一次进度
      store.progress = p;
      return;
    }
    job.progress = p;
    if (p.active) {
      if (p.paused && job.status === "running") job.status = "paused";
      if (!p.paused && job.status === "paused") job.status = "running";
    }
  });

  await onGameLog((line) => {
    store.log.push(line);
    if (store.log.length > 3000) store.log.splice(0, store.log.length - 3000);
  });

  await onGameExit((exit) => {
    store.gameRunning = false;
    const stopped = store.stoppedByUser;
    const versionId = store.runningVersionId;
    store.stoppedByUser = false;
    store.runningVersionId = null;
    if (exit.code !== 0 && !stopped && versionId) {
      notify(`游戏异常退出（代码 ${exit.code ?? "未知"}），正在分析日志`, "err");
      store.crashVersionId = versionId;
      store.crashExitCode = exit.code;
      store.crashOpen = true;
      return;
    }
    notify(
      exit.code === 0 ? "游戏已退出" : `游戏已退出（代码 ${exit.code ?? "未知"}）`,
      exit.code === 0 ? "ok" : "err"
    );
  });

  await onMsLoginAccount(async (account) => {
    if (!store.accounts.some((a) => a.id === account.id)) {
      store.accounts.push(account);
    }
    await refreshAccounts();
    await api.selectAccount(account.id);
    await refreshConfig();
    if (store.msLogin) store.msLogin.status = "ok";
    notify(`已登录：${account.name}`);
  });

  await onMsLogin((payload) => {
    if (store.msLogin) {
      store.msLogin.status = "err";
      store.msLogin.message = payload.message ?? "登录失败";
    }
  });
}
