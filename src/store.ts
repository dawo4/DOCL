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
  type DeviceCode,
  type InstalledVersion,
  type InstanceMeta,
  type JavaInfo,
  type LogLine,
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
    /// 与 kind 不同的实际安装分类，例如数据包需要带上目标世界
    installKind?: string;
  } | null,
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

export function applyTheme(theme?: string | null) {
  document.documentElement.dataset.theme = theme || "paper";
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

function primeProgress(stage: string, message: string) {
  store.progress = {
    stage,
    message,
    steps: [{ label: message, done: false }],
    files_done: 0,
    files_total: 0,
    bytes_done: 0,
    bytes_total: 0,
    elapsed_ms: 0,
    active: true,
  };
  store.lastProgress = store.progress;
}

export async function installVersion(id: string) {
  if (store.installingId) {
    notify("已有安装任务正在进行", "err");
    return;
  }
  store.installingId = id;
  store.progress = null;
  store.lastProgress = null;
  store.lastJobError = null;
  store.lastJobTitle = id;
  store.downloadPanel = "open";
  primeProgress("install", `准备安装 ${id}…`);
  try {
    await api.installVersion(id);
    await refreshInstalled();
    await refreshConfig();
    notify(`${id} 安装完成`);
  } catch (err) {
    store.lastJobError = String(err);
    notify(`安装失败：${String(err)}`, "err");
  } finally {
    store.installingId = null;
    window.setTimeout(() => {
      if (!store.progress?.active) store.progress = null;
    }, 800);
  }
}

export async function installLoader(
  versionId: string,
  loader: string,
  loaderVersion?: string
) {
  if (store.installingId) {
    notify("已有安装任务正在进行", "err");
    return;
  }
  store.installingId = `${versionId} · ${loader}`;
  store.progress = null;
  store.lastProgress = null;
  store.lastJobError = null;
  store.lastJobTitle = `${versionId} · ${loader}${loaderVersion ? ` ${loaderVersion}` : ""}`;
  store.downloadPanel = "open";
  primeProgress("install", `准备安装 ${loader}…`);
  try {
    await api.installLoader(versionId, loader, loaderVersion);
    await refreshInstalled();
    await refreshConfig();
    notify(`${loader} 安装完成`);
  } catch (err) {
    store.lastJobError = String(err);
    notify(`安装失败：${String(err)}`, "err");
  } finally {
    store.installingId = null;
    window.setTimeout(() => {
      if (!store.progress?.active) store.progress = null;
    }, 800);
  }
}

export async function installCfFile(
  versionId: string,
  kind: string,
  url: string,
  fileName: string,
  title: string
) {
  if (store.installingId) {
    notify("已有下载任务正在进行", "err");
    return;
  }
  store.installingId = `${versionId} · ${fileName}`;
  store.progress = null;
  store.lastProgress = null;
  store.lastJobError = null;
  store.lastJobTitle = title;
  store.downloadPanel = "open";
  primeProgress("install", `准备下载 ${title}…`);
  try {
    const result = await api.cfInstallFile(versionId, kind, url, fileName);
    const deps = result.dependencies;
    const notes: string[] = [];
    if (deps.installed.length) notes.push(`已自动补齐前置 ${deps.installed.join("、")}`);
    if (deps.failed.length) notes.push(`前置下载失败：${deps.failed.join("、")}`);
    notify(`${title} 下载完成${notes.length ? `，${notes.join("；")}` : ""}`);
    if (deps.unresolved.length) {
      store.lastJobError = `缺少前置：${deps.unresolved.join("、")}。当前实例的版本或加载器下没有找到对应文件，请手动安装。`;
    }
  } catch (err) {
    store.lastJobError = String(err);
    notify(`下载失败：${String(err)}`, "err");
  } finally {
    store.installingId = null;
    window.setTimeout(() => {
      if (!store.progress?.active) store.progress = null;
    }, 800);
  }
}

export async function downloadModpack(
  url: string,
  fileName: string,
  title: string
): Promise<string | null> {
  if (store.installingId) {
    notify("已有下载任务正在进行", "err");
    return null;
  }
  store.installingId = `整合包 · ${fileName}`;
  store.progress = null;
  store.lastProgress = null;
  store.lastJobError = null;
  store.lastJobTitle = title;
  store.downloadPanel = "open";
  primeProgress("install", `准备下载 ${title}…`);
  try {
    const path = await api.modpackDownload(url, fileName);
    notify(`${title} 下载完成`);
    return path;
  } catch (err) {
    store.lastJobError = String(err);
    notify(`下载失败：${String(err)}`, "err");
    return null;
  } finally {
    store.installingId = null;
    window.setTimeout(() => {
      if (!store.progress?.active) store.progress = null;
    }, 800);
  }
}

export async function importModpack(
  path: string,
  options: import("./api").ModpackImportOptions,
  title: string
) {
  if (store.installingId) {
    notify("已有下载任务正在进行", "err");
    return null;
  }
  store.installingId = `整合包 · ${title}`;
  store.progress = null;
  store.lastProgress = null;
  store.lastJobError = null;
  store.lastJobTitle = `导入整合包 · ${title}`;
  store.downloadPanel = "open";
  primeProgress("install", `准备导入 ${title}…`);
  try {
    const result = await api.modpackImport(path, options);
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
      store.lastJobError =
        result.failed.length > 5 ? `${shown} 等 ${result.failed.length} 个` : shown;
    }
    return result;
  } catch (err) {
    store.lastJobError = String(err);
    notify(`导入失败：${String(err)}`, "err");
    return null;
  } finally {
    store.installingId = null;
    window.setTimeout(() => {
      if (!store.progress?.active) store.progress = null;
    }, 800);
  }
}

export async function exportModpack(
  versionId: string,
  options: import("./api").ExportOptions,
  title: string
) {
  if (store.installingId) {
    notify("已有任务正在进行", "err");
    return null;
  }
  store.installingId = `导出 · ${title}`;
  store.progress = null;
  store.lastProgress = null;
  store.lastJobError = null;
  store.lastJobTitle = `导出整合包 · ${title}`;
  store.downloadPanel = "open";
  primeProgress("install", "准备导出整合包…");
  try {
    const result = await api.modpackExport(versionId, options);
    notify(`已导出到 ${result.paths[0] ?? options.target}`);
    return result;
  } catch (err) {
    store.lastJobError = String(err);
    notify(`导出失败：${String(err)}`, "err");
    return null;
  } finally {
    store.installingId = null;
    window.setTimeout(() => {
      if (!store.progress?.active) store.progress = null;
    }, 800);
  }
}

export async function installJava(major: number) {
  if (store.installingId) {
    notify("已有安装任务正在进行", "err");
    return;
  }
  store.installingId = `Java ${major}`;
  store.progress = null;
  store.lastProgress = null;
  store.lastJobError = null;
  store.lastJobTitle = `Java ${major}`;
  store.downloadPanel = "open";
  primeProgress("java", `准备安装 Java ${major}…`);
  try {
    const info = await api.installJava(major);
    await refreshJava();
    await refreshConfig();
    notify(`Java ${major} 已安装`);
    return info;
  } catch (err) {
    store.lastJobError = String(err);
    notify(`安装 Java 失败：${String(err)}`, "err");
    return null;
  } finally {
    store.installingId = null;
    window.setTimeout(() => {
      if (!store.progress?.active) store.progress = null;
    }, 800);
  }
}

export async function cancelInstall() {
  await api.cancelInstall();
  notify("已请求取消安装");
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
  } catch (err) {
    notify(String(err), "err");
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

/// Java 预检是异步的，期间挡住重复点击，避免同时拉起两个进程
let launchGuard = false;

export async function launchVersion(id?: string, opts?: { skipJavaCheck?: boolean }) {
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

export async function launchServer(address: string, id?: string, opts?: { skipJavaCheck?: boolean }) {
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
    api.systemMemory().then((value) => {
      store.systemMemory = value;
    }),
    api.defaultSkin().then((value) => {
      store.defaultSkin = value;
    }),
  ]);

  await onInstallProgress((p) => {
    store.progress = p;
    store.lastProgress = p;
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
