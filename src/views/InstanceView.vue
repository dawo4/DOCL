<script setup lang="ts">
import { computed, defineAsyncComponent, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { confirm, open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  api,
  openExternal,
  openFolder,
  pickWorldZip,
  revealFile,
  savePackDialog,
  type ExportPreview,
  type GameOptionsPreset,
  type PackUpdatePreview,
  type InstanceCheck,
  type ConfigEntry,
  type InstanceFile,
  type InstanceInfo,
  type InstanceMeta,
  type InstanceModMeta,
  type InstancePaths,
  type LoaderVersion,
  type RepairReport,
  type RepairPreview,
  type RepairBackup,
  type ModUpdate,
  type ModUpdateReport,
  type WorldSeed,
  type WorldBackup,
  type WorldSettings,
  type WorldSummary,
} from "../api";
import AppIcon from "../components/AppIcon.vue";
/// CodeMirror 相关包只在打开配置文件页时按需下载，不进入首屏
const ConfigCodeEditor = defineAsyncComponent(
  () => import("../components/ConfigCodeEditor.vue")
);
import IconButton from "../components/IconButton.vue";
import InstanceGlyph from "../components/InstanceGlyph.vue";
import WoodButton from "../components/WoodButton.vue";
import WoodInput from "../components/WoodInput.vue";
import WoodModal from "../components/WoodModal.vue";
import WoodSelect from "../components/WoodSelect.vue";
import WoodToggle from "../components/WoodToggle.vue";
import WorldSettingsModal from "../components/WorldSettingsModal.vue";
import {
  exportModpack,
  launchVersion,
  notify,
  openCommunity,
  openCrashDialog,
  refreshInstanceMetas,
  reinstallLoader,
  repairInstance,
  restoreRepairBackup,
  stopGame,
  store,
  updateLoader,
  updateModpack,
} from "../store";
import {
  ICON_PRESETS,
  formatBytes,
  formatDate,
  kindLabel,
  loaderLabel as loaderName,
} from "../utils";

type Tab =
  | "overview"
  | "launch"
  | "gameopts"
  | "mods"
  | "resourcepacks"
  | "shaderpacks"
  | "datapacks"
  | "schematics"
  | "saves"
  | "screenshots"
  | "config";

const tabs: { key: Tab; label: string; icon: string }[] = [
  { key: "overview", label: "概览", icon: "cube" },
  { key: "launch", label: "启动设置", icon: "settings" },
  { key: "mods", label: "Mod", icon: "grid" },
  { key: "resourcepacks", label: "资源包", icon: "image" },
  { key: "shaderpacks", label: "光影", icon: "sun" },
  { key: "datapacks", label: "数据包", icon: "package" },
  { key: "schematics", label: "投影", icon: "layers" },
  { key: "saves", label: "存档", icon: "database" },
  { key: "screenshots", label: "截图", icon: "folder" },
  { key: "gameopts", label: "游戏设置", icon: "sliders" },
  { key: "config", label: "配置文件", icon: "list" },
];

const contentTabs: Tab[] = [
  "mods",
  "resourcepacks",
  "shaderpacks",
  "datapacks",
  "schematics",
  "saves",
  "screenshots",
];

const tab = ref<Tab>("overview");
const info = ref<InstanceInfo | null>(null);
const paths = ref<InstancePaths | null>(null);
const loading = ref(true);
const saving = ref(false);
/// 回填表单时暂停自动保存，避免保存后再次触发
let formSyncing = false;
let metaTimer: number | undefined;
let metaPending = false;
const files = ref<InstanceFile[]>([]);
const filesLoading = ref(false);
const fileQuery = ref("");
const worlds = ref<WorldSummary[]>([]);
const datapackWorld = ref("");
const backups = ref<WorldBackup[]>([]);
const backupsLoading = ref(false);
const backupBusy = ref("");
const backupImporting = ref(false);
const togglingName = ref("");
const worldSettingsFolder = ref<string | null>(null);
const projectionMods = ref<{ litematica: boolean; worldedit: boolean } | null>(null);
const projectionLoading = ref(false);
const seedOpen = ref(false);
const seedBusy = ref(false);
const seedInfo = ref<WorldSeed | null>(null);
const confirmName = ref<string | null>(null);
let confirmTimer: number | undefined;

/* ---------- config 文本编辑器 ---------- */
const configEntries = ref<ConfigEntry[]>([]);
const configLoading = ref(false);
const configError = ref("");
const configQuery = ref("");
const configPath = ref("");
const configContent = ref("");
const configOriginal = ref("");
const configEncoding = ref("utf-8");
const configSize = ref(0);
const configSaving = ref(false);
/// 低内存模式不加载 CodeMirror，直接用原生 textarea，避免额外的主线程开销
const plainConfigEditor = computed(() => !!store.config?.low_memory);
/// 文件内容搜索走编辑器自带面板，这里只负责触发
const configEditorRef = ref<{ openSearch: () => void } | null>(null);
const configDirty = computed(() => configContent.value !== configOriginal.value);
/// 已展开的目录；默认只展开第一层，避免整棵树一次性铺开
const expandedDirs = ref<Set<string>>(new Set());
const configSearching = computed(() => configQuery.value.trim().length > 0);

function isDirExpanded(path: string) {
  return expandedDirs.value.has(path);
}

function toggleDir(path: string) {
  const next = new Set(expandedDirs.value);
  if (next.has(path)) next.delete(path);
  else next.add(path);
  expandedDirs.value = next;
}

/// 某一项是否可见：它的所有父级目录都处于展开状态
function isConfigVisible(path: string) {
  const parts = path.split("/");
  for (let i = 1; i < parts.length; i += 1) {
    if (!expandedDirs.value.has(parts.slice(0, i).join("/"))) return false;
  }
  return true;
}

const visibleConfigEntries = computed(() => {
  if (configSearching.value) {
    const keyword = configQuery.value.trim().toLowerCase();
    // 搜索时只列匹配的文件，不看折叠状态，也不显示目录
    return configEntries.value.filter(
      (entry) => !entry.is_dir && entry.path.toLowerCase().includes(keyword)
    );
  }
  return configEntries.value.filter((entry) => isConfigVisible(entry.path));
});

function configDepth(path: string) {
  return path.split("/").length - 1;
}

const modUpdates = ref<ModUpdateReport | null>(null);
const updatesLoading = ref(false);
const updateError = ref("");
const updatingKey = ref("");

const showExport = ref(false);
const exportPreview = ref<ExportPreview | null>(null);
const exportBusy = ref(false);
/// 用新版整合包文件更新实例
const packUpdateOpen = ref(false);
const packUpdateBusy = ref(false);
const packUpdating = ref(false);
const packUpdatePath = ref("");
const packUpdatePreview = ref<PackUpdatePreview | null>(null);
const packUpdateKindLabel: Record<string, string> = {
  added: "新增",
  changed: "更新",
  removed: "移除",
};
const exportForm = reactive({
  name: "",
  version: "1.0.0",
  format: "zip",
  groups: [] as string[],
  target: "",
  extras: [] as string[],
});
/// 用户手动添加的额外目录（TACZ 枪包等）
const customExtras = ref<string[]>([]);
const customExtraInput = ref("");

function toggleExtra(dir: string) {
  const index = exportForm.extras.indexOf(dir);
  if (index >= 0) {
    exportForm.extras.splice(index, 1);
  } else {
    exportForm.extras.push(dir);
  }
}

function addCustomExtra() {
  const value = customExtraInput.value.trim().replace(/\\/g, "/").replace(/^\/+|\/+$/g, "");
  if (!value) return;
  if (value.startsWith("..") || /^[a-zA-Z]:/.test(value)) {
    notify("请输入实例内的相对路径，例如 tacz", "err");
    return;
  }
  if (
    exportForm.extras.includes(value) ||
    customExtras.value.includes(value) ||
    (exportPreview.value?.suggested_extras ?? []).some((entry) => entry.key === value)
  ) {
    notify("该目录已经在列表中", "err");
    return;
  }
  customExtras.value.push(value);
  customExtraInput.value = "";
}

const exportFormats = [
  { value: "zip", label: "通用 ZIP（兼容各启动器）" },
  { value: "mrpack", label: "Modrinth .mrpack" },
];

const iconPresets = ICON_PRESETS;

const categoryOptions = [
  { value: "auto", label: "自动" },
  { value: "survival", label: "生存" },
  { value: "creative", label: "创造" },
  { value: "pvp", label: "PvP" },
  { value: "modpack", label: "整合包" },
  { value: "tech", label: "生电" },
  { value: "build", label: "建筑" },
  { value: "other", label: "其他" },
];

const fullscreenOptions = [
  { value: "inherit", label: "跟随全局" },
  { value: "on", label: "开启全屏" },
  { value: "off", label: "窗口模式" },
];

const memoryModeOptions = [
  { value: "inherit", label: "跟随全局" },
  { value: "auto", label: "自动（G1GC）" },
  { value: "g1gc", label: "调优 G1GC" },
  { value: "zgc", label: "ZGC（低暂停）" },
  { value: "custom", label: "自定义" },
];

const wrapperOptions = [
  { value: "inherit", label: "跟随全局" },
  { value: "on", label: "注入标识" },
  { value: "off", label: "禁用 Wrapper" },
];

const unsafeOptions = [
  { value: "inherit", label: "跟随全局" },
  { value: "on", label: "启用兼容补丁" },
  { value: "off", label: "禁用兼容补丁" },
];

const gpuOptions = [
  { value: "inherit", label: "跟随全局" },
  { value: "on", label: "使用高性能显卡" },
  { value: "off", label: "不使用" },
];

const form = reactive({
  display_name: "",
  description: "",
  icon: "auto",
  category: "auto",
  favorite: false,
  memory: "",
  javaPath: "",
  javaArgs: "",
  memoryMode: "inherit",
  gameArgs: "",
  preLaunch: "",
  wrapper: "inherit",
  unsafe: "inherit",
  gpu: "inherit",
  width: "",
  height: "",
  fullscreen: "inherit",
});

const javaOptions = computed(() => [
  { value: "", label: "跟随全局设置" },
  ...store.javaList.map((java) => ({
    value: java.path,
    label: `Java ${java.major} · ${java.vendor} ${java.kind}`,
  })),
]);

/// 自动内存的推荐值由后端按「当前可用内存」实时计算（参考 PCL）
const recommendedMemory = computed(() => store.systemMemory.recommended_mb || 4096);

const iconColor = computed(
  () => iconPresets.find((preset) => preset.value === form.icon)?.color ?? "#8b9d6f"
);

/// form.icon 为 custom 时对应图片的 data URL
const customIcon = ref<string | null>(null);

async function loadCustomIcon() {
  if (!store.instanceId || form.icon !== "custom") {
    customIcon.value = null;
    return;
  }
  try {
    customIcon.value = await api.instanceIconImage(store.instanceId);
  } catch {
    customIcon.value = null;
  }
}

watch(() => form.icon, loadCustomIcon);

/* ---------- 游戏设置预设：把 options.txt 存成模板，手动应用到版本 ---------- */
const optionsPresets = ref<GameOptionsPreset[]>([]);
const optionsName = ref("");
const optionsBusy = ref(false);

async function loadOptionsPresets() {
  try {
    optionsPresets.value = await api.gameOptionsPresets();
  } catch (err) {
    notify(`读取游戏设置预设失败：${String(err)}`, "err");
  }
}

function presetDate(preset: GameOptionsPreset) {
  return formatDate(new Date(preset.saved_at * 1000).toISOString());
}

async function saveOptionsPreset() {
  if (!store.instanceId || optionsBusy.value) return;
  optionsBusy.value = true;
  try {
    const saved = await api.saveGameOptionsPreset(
      store.instanceId,
      optionsName.value.trim() || "我的游戏设置"
    );
    await loadOptionsPresets();
    optionsName.value = "";
    notify(`已保存游戏设置预设「${saved.name}」`);
  } catch (err) {
    notify(String(err), "err");
  } finally {
    optionsBusy.value = false;
  }
}

async function applyOptionsPreset(preset: GameOptionsPreset) {
  if (!store.instanceId || optionsBusy.value) return;
  // 游戏退出时会重写 options.txt，运行中应用只会白改
  if (store.gameRunning) {
    notify("游戏正在运行：请先退出游戏，否则退出时会覆盖刚应用的设置", "err");
    return;
  }
  optionsBusy.value = true;
  try {
    await api.applyGameOptionsPreset(store.instanceId, preset.id);
    notify(`已应用「${preset.name}」，原设置已备份为 options.txt.bak`);
  } catch (err) {
    notify(String(err), "err");
  } finally {
    optionsBusy.value = false;
  }
}

async function deleteOptionsPreset(preset: GameOptionsPreset) {
  const ok = await confirm(`确定删除预设「${preset.name}」？`, {
    title: "删除游戏设置预设",
    kind: "warning",
  });
  if (!ok) return;
  try {
    await api.deleteGameOptionsPreset(preset.id);
    await loadOptionsPresets();
    notify("已删除游戏设置预设");
  } catch (err) {
    notify(`删除失败：${String(err)}`, "err");
  }
}

async function pickIconImage() {
  if (!store.instanceId) return;
  const source = await openDialog({
    multiple: false,
    title: "选择版本图标",
    filters: [{ name: "图片", extensions: ["png", "jpg", "jpeg", "webp", "gif", "bmp"] }],
  });
  if (!source || typeof source !== "string") return;
  try {
    await api.instanceSetIconImage(store.instanceId, source);
    form.icon = "custom";
    await loadCustomIcon();
    await refreshInstanceMetas();
    notify("已更新版本图标");
  } catch (err) {
    notify(`图标保存失败：${String(err)}`, "err");
  }
}

const loaderLabel = computed(() => loaderName(info.value?.loader));

const isContentTab = computed(() => contentTabs.includes(tab.value));
const contentLabel = computed(
  () => tabs.find((item) => item.key === tab.value)?.label ?? ""
);
const contentKind = computed(() => {
  if (!isContentTab.value) return "";
  if (tab.value === "datapacks") {
    return datapackWorld.value ? `datapacks:${datapackWorld.value}` : "datapacks";
  }
  return tab.value as string;
});
const worldOptions = computed(() =>
  worlds.value.map((world) => ({ value: world.folder, label: world.name }))
);
const selectedWorld = computed(() =>
  worlds.value.find((world) => world.folder === datapackWorld.value)
);
const noWorlds = computed(() => tab.value === "datapacks" && !worlds.value.length);
const projectionStatus = computed(() => {
  if (projectionLoading.value) return "正在检测已安装的投影 Mod…";
  if (!projectionMods.value) return "";
  const found: string[] = [];
  if (projectionMods.value.litematica) found.push("Litematica");
  if (projectionMods.value.worldedit) found.push("WorldEdit");
  return found.length ? `已检测到 ${found.join(" / ")}` : "未检测到 Litematica 或 WorldEdit";
});
const filteredFiles = computed(() => {
  const keyword = fileQuery.value.trim().toLowerCase();
  if (!keyword) return files.value;
  return files.value.filter((file) =>
    file.display_name.toLowerCase().includes(keyword)
  );
});

/// 一次最多渲染的条数：几百个 Mod 全量挂 DOM 会明显抬高渲染进程的内存，
/// 其余条目点「显示更多」再追加，不影响搜索与排序。
const FILE_PAGE_SIZE = 120;
const fileLimit = ref(FILE_PAGE_SIZE);
const visibleFiles = computed(() => filteredFiles.value.slice(0, fileLimit.value));
const hiddenFileCount = computed(() =>
  Math.max(filteredFiles.value.length - visibleFiles.value.length, 0)
);

function showMoreFiles() {
  fileLimit.value += FILE_PAGE_SIZE;
}

// 切换分类、改动搜索词或换实例时，回到第一页
watch([tab, fileQuery, () => store.instanceId], () => {
  fileLimit.value = FILE_PAGE_SIZE;
});

const canDownloadMore = computed(
  () =>
    tab.value === "mods" ||
    tab.value === "resourcepacks" ||
    tab.value === "shaderpacks" ||
    tab.value === "datapacks"
);

const contentHint = computed(() => {
  switch (tab.value) {
    case "mods":
      return "禁用会保留文件并加上 .disabled 后缀，游戏不会加载";
    case "resourcepacks":
      return "支持 .zip 资源包与文件夹形式的资源包";
    case "shaderpacks":
      return "光影包需要游戏内安装对应光影加载器";
    case "datapacks":
      return "数据包只对选中的存档世界生效，安装后在世界里执行 /reload 即可加载";
    case "schematics":
      return "投影文件本身不能直接生效，需要客户端安装 Litematica（.litematic）或 WorldEdit（.schem / .schematic）";
    case "saves":
      return "每个文件夹就是一个存档世界";
    case "screenshots":
      return "游戏内截图会保存在这里";
    default:
      return "";
  }
});

onMounted(async () => {
  if (!store.instanceId) {
    store.page = "versions";
    return;
  }
  await loadAll();
  await loadCustomIcon();
  await loadOptionsPresets();
  window.addEventListener("keydown", onConfigFind);
});

onBeforeUnmount(() => {
  window.removeEventListener("keydown", onConfigFind);
});

/// 配置文件页按 Ctrl/Cmd + F 直接打开内容搜索，避免落到浏览器默认查找
function onConfigFind(event: KeyboardEvent) {
  if (tab.value !== "config") return;
  if (!(event.ctrlKey || event.metaKey) || event.key.toLowerCase() !== "f") return;
  if (!configPath.value || plainConfigEditor.value) return;
  event.preventDefault();
  configEditorRef.value?.openSearch();
}

watch(
  () => store.contentRevision,
  () => {
    if (!store.instanceId) return;
    void loadAll();
    if (contentTabs.includes(tab.value)) void loadFiles();
    if (tab.value === "saves") void loadBackups();
  }
);

watch(tab, async (value) => {
  fileQuery.value = "";
  confirmName.value = null;
  if (value === "gameopts") void loadOptionsPresets();
  if (!contentTabs.includes(value)) return;
  if (value === "schematics") void loadProjectionMods();
  if (value === "saves") void loadBackups();
  if (value === "datapacks") {
    await loadWorlds();
    if (!datapackWorld.value) {
      files.value = [];
      return;
    }
  }
  await loadFiles();
});

async function loadWorlds() {
  if (!store.instanceId) return;
  try {
    const list = await api.instanceWorlds(store.instanceId);
    worlds.value = list;
    if (!list.some((world) => world.folder === datapackWorld.value)) {
      datapackWorld.value = list[0]?.folder ?? "";
    }
  } catch (err) {
    worlds.value = [];
    datapackWorld.value = "";
    notify(`读取存档列表失败：${String(err)}`, "err");
  }
}

async function loadProjectionMods() {
  if (!store.instanceId) return;
  projectionLoading.value = true;
  try {
    const list = await api.instanceFiles(store.instanceId, "mods");
    const names = list.map((file) => file.name.toLowerCase());
    projectionMods.value = {
      litematica: names.some((name) => name.includes("litematica")),
      worldedit: names.some((name) => name.includes("worldedit")),
    };
  } catch {
    projectionMods.value = null;
  } finally {
    projectionLoading.value = false;
  }
}

async function switchWorld(value: string) {
  if (datapackWorld.value === value) return;
  datapackWorld.value = value;
  fileQuery.value = "";
  await loadFiles();
}

async function loadAll() {
  loading.value = true;
  try {
    const id = store.instanceId!;
    const [detail, folder] = await Promise.all([
      api.instanceInfo(id),
      api.instancePaths(id),
    ]);
    info.value = detail;
    paths.value = folder;
    modUpdates.value = null;
    updateError.value = "";
    projectionMods.value = null;
    applyForm(detail);
  } catch (err) {
    notify(`读取版本信息失败：${String(err)}`, "err");
    store.page = "versions";
  } finally {
    loading.value = false;
  }
}

function applyForm(detail: InstanceInfo) {
  formSyncing = true;
  form.display_name =
    detail.display_name && detail.display_name !== detail.id
      ? detail.display_name
      : "";
  form.description = detail.description ?? "";
  form.icon = detail.icon || "auto";
  form.category = detail.category || "auto";
  form.favorite = detail.favorite;
  form.memory = detail.memory_mb ? String(detail.memory_mb) : "";
  form.javaPath = detail.java_path ?? "";
  form.javaArgs = detail.java_args ?? "";
  form.memoryMode = detail.memory_mode ?? "inherit";
  form.gameArgs = detail.game_args ?? "";
  form.preLaunch = detail.pre_launch ?? "";
  form.wrapper =
    detail.disable_launch_wrapper === null
      ? "inherit"
      : detail.disable_launch_wrapper
        ? "off"
        : "on";
  form.unsafe =
    detail.disable_lwjgl_unsafe === null
      ? "inherit"
      : detail.disable_lwjgl_unsafe
        ? "off"
        : "on";
  form.gpu =
    detail.prefer_gpu === null ? "inherit" : detail.prefer_gpu ? "on" : "off";
  form.width = detail.window_width ? String(detail.window_width) : "";
  form.height = detail.window_height ? String(detail.window_height) : "";
  form.fullscreen =
    detail.fullscreen === null ? "inherit" : detail.fullscreen ? "on" : "off";
  void nextTick(() => {
    formSyncing = false;
  });
}

/// 任意字段改动后防抖保存，不再需要手动点保存
watch(
  () => ({ ...form }),
  () => {
    if (formSyncing || !store.instanceId) return;
    window.clearTimeout(metaTimer);
    metaTimer = window.setTimeout(() => void saveMeta(true), 700);
  }
);

function numberOrNull(value: string, min: number) {
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed >= min ? Math.round(parsed) : null;
}

function buildMeta(): InstanceMeta {
  return {
    display_name: form.display_name.trim() || null,
    description: form.description.trim(),
    icon: form.icon,
    category: form.category,
    favorite: form.favorite,
    memory_mb: numberOrNull(form.memory, 512),
    memory_mode: form.memoryMode === "inherit" ? null : form.memoryMode,
    java_path: form.javaPath || null,
    java_args: form.javaArgs.trim() || null,
    game_args: form.gameArgs.trim() || null,
    pre_launch: form.preLaunch.trim() || null,
    disable_launch_wrapper: form.wrapper === "inherit" ? null : form.wrapper === "off",
    disable_lwjgl_unsafe: form.unsafe === "inherit" ? null : form.unsafe === "off",
    prefer_gpu: form.gpu === "inherit" ? null : form.gpu === "on",
    window_width: numberOrNull(form.width, 320),
    window_height: numberOrNull(form.height, 240),
    fullscreen:
      form.fullscreen === "inherit" ? null : form.fullscreen === "on",
  };
}

async function saveMeta(silent = false) {
  if (!store.instanceId) return;
  window.clearTimeout(metaTimer);
  if (saving.value) {
    metaPending = true;
    return;
  }
  saving.value = true;
  try {
    const detail = await api.instanceSaveMeta(store.instanceId, buildMeta());
    info.value = detail;
    applyForm(detail);
    await refreshInstanceMetas();
    if (!silent) notify("版本设置已保存");
  } catch (err) {
    notify(`保存失败：${String(err)}`, "err");
  } finally {
    saving.value = false;
    if (metaPending) {
      metaPending = false;
      window.clearTimeout(metaTimer);
      metaTimer = window.setTimeout(() => void saveMeta(true), 700);
    }
  }
}

function toggleFavorite() {
  form.favorite = !form.favorite;
}

async function loadFiles() {
  if (!store.instanceId) return;
  // 概览/启动设置等非内容页也会触发刷新，别把空分类发给后端
  if (!contentKind.value) return;
  if (tab.value === "datapacks" && !datapackWorld.value) {
    files.value = [];
    return;
  }
  filesLoading.value = true;
  try {
    files.value = await api.instanceFiles(store.instanceId, contentKind.value);
    void loadThumbs();
    void loadModMetas();
  } catch (err) {
    notify(`读取文件失败：${String(err)}`, "err");
    files.value = [];
  } finally {
    filesLoading.value = false;
  }
}

/// 已识别来源的模组：文件名（去 .disabled、小写）→ 项目信息
const modMetas = ref<Record<string, InstanceModMeta>>({});

async function loadModMetas() {
  if (!store.instanceId || contentKind.value !== "mods") return;
  const versionId = store.instanceId;
  try {
    const list = await api.instanceModMeta(versionId);
    if (store.instanceId !== versionId) return;
    const map: Record<string, InstanceModMeta> = {};
    for (const item of list) {
      map[item.file_name.toLowerCase()] = item;
    }
    modMetas.value = map;
  } catch {
    // 识别失败不影响模组列表
  }
}

function modMetaOf(file: InstanceFile) {
  if (contentKind.value !== "mods") return undefined;
  return modMetas.value[file.name.replace(/\.disabled$/i, "").toLowerCase()];
}

function openModSource(file: InstanceFile) {
  const meta = modMetaOf(file);
  if (meta?.website_url) openExternal(meta.website_url);
}

const thumbs = reactive<Record<string, string>>({});
const thumbPending = new Set<string>();

function thumbKey(file: InstanceFile) {
  return `${contentKind.value}:${file.name}:${file.modified}`;
}

function thumbOf(file: InstanceFile) {
  return thumbs[thumbKey(file)] ?? "";
}

/// 逐张按需生成缩略图：只处理当前渲染出来的条目，
/// 否则一个几百张截图的目录会把所有 base64 图片常驻在内存里。
async function loadThumbs() {
  if (!store.instanceId || !["saves", "screenshots"].includes(contentKind.value)) return;
  const versionId = store.instanceId;
  const kind = contentKind.value;
  for (const file of visibleFiles.value) {
    if (kind === "screenshots" && file.is_dir) continue;
    if (kind === "saves" && !file.is_dir) continue;
    const key = `${kind}:${file.name}:${file.modified}`;
    if (thumbs[key] || thumbPending.has(key)) continue;
    thumbPending.add(key);
    try {
      const data = await api.instanceThumb(versionId, kind, file.name);
      if (data && store.instanceId === versionId && contentKind.value === kind) {
        thumbs[key] = data;
      }
    } catch {
      // 单张缩略图失败不影响列表
    } finally {
      thumbPending.delete(key);
    }
  }
}

// 切换分类/实例时释放旧缩略图；翻页或搜索后补加载新出现的条目
watch([tab, fileQuery, () => store.instanceId], () => {
  for (const key of Object.keys(thumbs)) delete thumbs[key];
  thumbPending.clear();
});

watch(visibleFiles, () => {
  if (["saves", "screenshots"].includes(contentKind.value)) void loadThumbs();
});

async function refreshFiles() {
  await loadFiles();
  if (store.instanceId) {
    try {
      info.value = await api.instanceInfo(store.instanceId);
    } catch {
      // 忽略计数刷新失败
    }
  }
}

async function checkModUpdates() {
  if (!info.value || updatesLoading.value) return;
  updatesLoading.value = true;
  updateError.value = "";
  try {
    modUpdates.value = await api.modUpdates(info.value.id);
  } catch (err) {
    modUpdates.value = null;
    updateError.value = String(err);
  } finally {
    updatesLoading.value = false;
  }
}

async function applyModUpdate(item: ModUpdate) {
  if (!info.value || updatingKey.value) return false;
  updatingKey.value = item.file_name;
  try {
    await api.modUpdateInstall(
      info.value.id,
      item.download_url,
      item.latest_file,
      item.file_name,
      item.sha1
    );
    notify(`${item.mod_name} 已更新到 ${item.latest_version}`);
    if (modUpdates.value) {
      modUpdates.value.updates = modUpdates.value.updates.filter(
        (row) => row.file_name !== item.file_name
      );
      modUpdates.value.up_to_date += 1;
    }
    await refreshFiles();
    return true;
  } catch (err) {
    notify(`更新 ${item.mod_name} 失败：${String(err)}`, "err");
    return false;
  } finally {
    updatingKey.value = "";
  }
}

async function applyAllModUpdates() {
  const list = modUpdates.value ? [...modUpdates.value.updates] : [];
  for (const item of list) {
    const ok = await applyModUpdate(item);
    if (!ok) break;
  }
}

async function openContentFolder() {
  if (!isContentTab.value) return;
  if (tab.value === "datapacks") {
    const target = selectedWorld.value?.datapacks;
    if (target) openFolder(target);
    return;
  }
  if (!paths.value) return;
  const target = paths.value[tab.value as keyof InstancePaths];
  if (target) openFolder(target);
}

function openVersionFolder() {
  if (paths.value) openFolder(paths.value.version);
}

function openGameFolder() {
  if (paths.value) openFolder(paths.value.game_dir);
}

const CHECK_KIND_LABELS: Record<string, string> = {
  mods: "Mod",
  resourcepacks: "资源包",
  shaderpacks: "光影",
};

const checkOpen = ref(false);
const checking = ref(false);
const checkResult = ref<InstanceCheck | null>(null);

const repairOpen = ref(false);
const repairing = ref(false);
const repairResult = ref<RepairReport | null>(null);
const reinstalling = ref(false);
/// 修复前的可选项：下载量大或会改设置的动作默认关闭
const repairOptionsOpen = ref(false);
const repairOptions = ref({
  install_deps: false,
  reset_graphics: false,
  repair_assets: false,
  reinstall_loader: false,
});
/// 修复计划：会改动的具体文件，勾选项变化后重新扫描
const repairPreview = ref<RepairPreview | null>(null);
const repairPreviewing = ref(false);
/// 连续切换勾选项时只认最后一次请求，避免旧结果覆盖新选择
let previewToken = 0;
let previewTimer: ReturnType<typeof setTimeout> | undefined;

async function loadRepairPreview() {
  const versionId = store.instanceId;
  if (!versionId) return;
  const token = ++previewToken;
  repairPreviewing.value = true;
  try {
    const result = await api.previewRepair(versionId, repairOptions.value);
    if (token === previewToken) repairPreview.value = result;
  } catch {
    if (token === previewToken) repairPreview.value = null;
  } finally {
    if (token === previewToken) repairPreviewing.value = false;
  }
}

function openRepairOptions() {
  repairOptionsOpen.value = true;
  void loadRepairPreview();
}

watch(
  repairOptions,
  () => {
    if (!repairOptionsOpen.value) return;
    // 扫描 mods 目录可能较慢，连着勾选时合并成一次
    if (previewTimer) clearTimeout(previewTimer);
    previewTimer = setTimeout(() => void loadRepairPreview(), 250);
  },
  { deep: true }
);

/// 只对标准加载器实例显示重装入口，整合包重装会破坏包内清单
const canReinstallLoader = computed(() => {
  const value = info.value;
  if (!value || value.category === "modpack") return false;
  return ["forge", "neoforge", "fabric", "quilt"].includes(value.loader);
});

/// 重装加载器：清理损坏的版本目录后按原加载器版本重新安装
async function reinstallCurrentLoader() {
  const versionId = store.instanceId;
  if (!versionId || reinstalling.value) return;
  const ok = await confirm(
    "将删除并重新生成版本清单与补丁 jar，不会删除 mods、存档和配置。继续吗？",
    { title: "重装加载器", kind: "warning" }
  );
  if (!ok) return;
  reinstalling.value = true;
  try {
    const done = await reinstallLoader(versionId);
    if (done) {
      repairOpen.value = false;
      await refreshFiles();
    }
  } finally {
    reinstalling.value = false;
  }
}

/// 加载器版本更新：列表与选择状态
const loaderUpdateOpen = ref(false);
const loaderUpdateLoading = ref(false);
const loaderUpdateError = ref("");
const loaderUpdateTarget = ref("");
const loaderUpdateVersions = ref<LoaderVersion[]>([]);
const updatingLoader = ref(false);
/// 版本列表按「游戏版本:加载器」缓存，切换实例后不会串用
const loaderUpdateKey = ref("");

// 切换实例时关掉更新弹窗，避免把 A 实例的选择应用到 B 实例
watch(
  () => store.instanceId,
  () => {
    loaderUpdateOpen.value = false;
  }
);

async function openLoaderUpdate() {
  const value = info.value;
  if (!value || !canReinstallLoader.value) return;
  loaderUpdateOpen.value = true;
  loaderUpdateError.value = "";
  loaderUpdateTarget.value = "";
  const key = `${value.game_version}:${value.loader}`;
  if (loaderUpdateVersions.value.length && loaderUpdateKey.value === key) {
    pickDefaultLoaderVersion();
    return;
  }
  loaderUpdateLoading.value = true;
  try {
    loaderUpdateVersions.value = await api.loaderVersions(value.game_version, value.loader);
    loaderUpdateKey.value = key;
    pickDefaultLoaderVersion();
  } catch (err) {
    loaderUpdateError.value = String(err);
  } finally {
    loaderUpdateLoading.value = false;
  }
}

/// 默认选推荐版本；没有推荐时取最新（列表已按版本倒序）
function pickDefaultLoaderVersion() {
  const list = loaderUpdateVersions.value;
  const pick = list.find((item) => item.recommended) ?? list[0];
  loaderUpdateTarget.value = pick?.version ?? "";
}

async function applyLoaderUpdate() {
  const versionId = store.instanceId;
  if (!versionId || !loaderUpdateTarget.value || updatingLoader.value) return;
  // 运行中的游戏会锁住补丁 jar，替换必然失败
  if (store.gameRunning && store.runningVersionId === versionId) {
    notify("请先关闭游戏，再更新加载器", "err");
    return;
  }
  updatingLoader.value = true;
  try {
    const done = await updateLoader(versionId, loaderUpdateTarget.value);
    if (done) {
      loaderUpdateOpen.value = false;
      await loadAll();
    }
  } finally {
    updatingLoader.value = false;
  }
}

/// 一键自愈：修清单、补缺失文件、重解本地库、禁用重复模组
async function runRepair() {
  const versionId = store.instanceId;
  if (repairing.value || !versionId) return;
  repairing.value = true;
  repairResult.value = null;
  try {
    const result = await repairInstance(versionId, repairOptions.value);
    if (!result) return;
    repairResult.value = result;
    repairOpen.value = true;
    await refreshFiles();
    if (result.disabled_mods.length || result.incompatible_mods.length)
      notify("已禁用有问题的模组，重新启动后生效");
    else if (!result.files_restored && !result.issues.length) notify("检查完成，没有发现问题");
  } finally {
    repairing.value = false;
  }
}

/// 修复结果里需要展示的行
const repairLines = computed(() => {
  const result = repairResult.value;
  if (!result) return [];
  const lines: string[] = [];
  if (result.version_json) lines.push("版本清单已重新下载修复");
  if (result.files_restored) lines.push(`补回 ${result.files_restored} 个缺失文件`);
  if (result.natives) lines.push("本地库已重新解压");
  if (result.disabled_mods.length)
    lines.push(`禁用了 ${result.disabled_mods.length} 个重复模组：${result.disabled_mods.slice(0, 4).join("、")}`);
  if (result.incompatible_mods.length)
    lines.push(
      `禁用了 ${result.incompatible_mods.length} 个不兼容模组：${result.incompatible_mods
        .slice(0, 4)
        .join("、")}（可换 ForgeV2 / ForgeActive 版本，或直接不装）`
    );
  if (result.missing_deps.length)
    lines.push(`缺少前置模组：${result.missing_deps.slice(0, 6).join("、")}`);
  if (result.broken_mods?.length)
    lines.push(
      `发现 ${result.broken_mods.length} 个损坏的 Mod 文件：${result.broken_mods
        .slice(0, 4)
        .join("、")}`
    );
  if (result.deps_installed?.length)
    lines.push(`已自动补装前置：${result.deps_installed.slice(0, 5).join("、")}`);
  if (result.java_switched) lines.push(`已切换 Java：${result.java_switched}`);
  if (result.graphics_reset) lines.push("画质相关选项已调低（渲染距离 / 阴影 / 粒子）");
  if (result.assets_restored)
    lines.push(`补全了 ${result.assets_restored} 个资源文件`);
  if (result.config_reset) lines.push("已备份并清理损坏的 config，Mod 会重新生成配置");
  if (result.quarantined_mods?.length)
    lines.push(
      `隔离了 ${result.quarantined_mods.length} 个解压残留：${result.quarantined_mods
        .slice(0, 4)
        .join("、")}`
    );
  if (result.renamed_mods?.length)
    lines.push(`修正了 ${result.renamed_mods.length} 个非法的 Mod 文件名`);
  if (result.installed_fixes?.length)
    lines.push(`已安装修复类 Mod：${result.installed_fixes.join("、")}`);
  if (result.removed_paths?.length)
    lines.push(
      `已清理 ${result.removed_paths.length} 个被占用的文件：${result.removed_paths
        .slice(0, 4)
        .join("、")}`
    );
  if (result.removed_java_agents?.length)
    lines.push(
      `已移除 ${result.removed_java_agents.length} 个无效的 -javaagent 参数`
    );
  if (result.loader_reinstalled) lines.push("已重装加载器（mods 与存档保留）");
  return lines;
});

/// 还原本次修复改动过的文件
const restoring = ref(false);
async function undoRepair() {
  const versionId = store.instanceId;
  if (!versionId || restoring.value) return;
  restoring.value = true;
  try {
    const count = await restoreRepairBackup(versionId);
    if (count > 0) {
      repairOpen.value = false;
      await refreshFiles();
      notify("已还原到修复前的状态，重新启动后生效");
    }
  } finally {
    restoring.value = false;
  }
}

/// 修复结果里仍然需要手动处理的问题
const repairProblems = computed(() => {
  const result = repairResult.value;
  if (!result) return [];
  return [...result.issues, ...result.files_failed];
});

/// 修复历史：每次修复前的备份，可按次还原
const historyOpen = ref(false);
const history = ref<RepairBackup[]>([]);
const historyLoading = ref(false);
const restoringBackupId = ref("");

async function openRepairHistory() {
  const versionId = store.instanceId;
  if (!versionId) return;
  historyOpen.value = true;
  historyLoading.value = true;
  try {
    history.value = await api.repairBackups(versionId);
  } catch (err) {
    history.value = [];
    notify(`读取修复历史失败：${String(err)}`, "err");
  } finally {
    historyLoading.value = false;
  }
}

function formatBackupTime(seconds: number) {
  if (!seconds) return "时间未知";
  return new Date(seconds * 1000).toLocaleString("zh-CN", { hour12: false });
}

async function restoreBackupAt(entry: RepairBackup) {
  const versionId = store.instanceId;
  if (!versionId || restoringBackupId.value) return;
  // 先占位再等确认，避免连点触发两次还原
  restoringBackupId.value = entry.id;
  try {
    const ok = await confirm(
      `会把 ${entry.files} 个文件还原到 ${formatBackupTime(entry.created)} 的状态，之后修复产生的改动会被覆盖。继续吗？`,
      { title: "还原这次修复", kind: "warning" }
    );
    if (!ok) return;
    const count = await api.restoreRepairBackupAt(versionId, entry.id);
    if (count > 0) {
      historyOpen.value = false;
      repairOpen.value = false;
      repairOptionsOpen.value = false;
      await refreshFiles();
      notify(`已还原 ${count} 个文件，重新启动后生效`);
    } else {
      notify("这次备份里没有可还原的文件");
    }
  } catch (err) {
    notify(`还原失败：${String(err)}`, "err");
  } finally {
    restoringBackupId.value = "";
  }
}

/// 扫描 mods / 资源包 / 光影里打不开的压缩包。
/// FML 初始化时遇到损坏的 jar 会直接退出，这个入口用来给已装好的实例排查。
async function runSelfCheck() {
  const versionId = store.instanceId;
  if (checking.value || !versionId) return;
  checking.value = true;
  checkResult.value = null;
  checkOpen.value = true;
  try {
    const result = await api.checkInstanceFiles(versionId);
    checkResult.value = result;
    if (result.broken.length === 0) {
      notify(`自检完成：${result.scanned} 个文件全部正常`);
    }
  } catch (err) {
    notify(`自检失败：${String(err)}`, "err");
    checkOpen.value = false;
  } finally {
    checking.value = false;
  }
}

function brokenFolder(kind: string): string | null {
  const value = paths.value;
  if (!value) return null;
  if (kind === "mods") return value.mods;
  if (kind === "resourcepacks") return value.resourcepacks;
  if (kind === "shaderpacks") return value.shaderpacks;
  return null;
}

function openBrokenFolder() {
  const first = checkResult.value?.broken[0];
  const folder = first ? brokenFolder(first.kind) : null;
  if (folder) openFolder(folder);
}

async function toggleFile(file: InstanceFile) {
  if (!store.instanceId || togglingName.value) return;
  togglingName.value = file.name;
  try {
    await api.instanceToggleFile(store.instanceId, contentKind.value, file.name);
    await loadFiles();
  } catch (err) {
    notify(`切换失败：${String(err)}`, "err");
  } finally {
    togglingName.value = "";
  }
}

async function openShot(file: InstanceFile) {
  if (!store.instanceId) return;
  try {
    await api.instanceOpenContent(store.instanceId, "screenshots", file.name);
  } catch (err) {
    notify(`打开截图失败：${String(err)}`, "err");
  }
}

async function openSeed(file: InstanceFile) {
  if (!store.instanceId) return;
  seedOpen.value = true;
  seedBusy.value = true;
  seedInfo.value = null;
  try {
    seedInfo.value = await api.instanceWorldSeed(store.instanceId, file.name);
  } catch (err) {
    seedOpen.value = false;
    notify(`读取种子失败：${String(err)}`, "err");
  } finally {
    seedBusy.value = false;
  }
}

async function copySeed() {
  if (!seedInfo.value?.seed) return;
  try {
    await navigator.clipboard.writeText(seedInfo.value.seed);
    notify("种子已复制");
  } catch (err) {
    notify(`复制失败：${String(err)}`, "err");
  }
}

function openSeedMap() {
  if (!seedInfo.value?.seed) return;
  openExternal(
    `https://www.chunkbase.com/apps/seed-map#seed=${encodeURIComponent(seedInfo.value.seed)}`
  );
}

function requestDelete(file: InstanceFile) {
  if (confirmName.value === file.name) {
    confirmName.value = null;
    removeFile(file);
    return;
  }
  confirmName.value = file.name;
  if (confirmTimer) window.clearTimeout(confirmTimer);
  confirmTimer = window.setTimeout(() => {
    confirmName.value = null;
  }, 3200);
}

async function removeFile(file: InstanceFile) {
  if (!store.instanceId) return;
  try {
    await api.instanceDeleteFile(store.instanceId, contentKind.value, file.name);
    await refreshFiles();
    notify(`已删除 ${file.display_name}`);
  } catch (err) {
    notify(`删除失败：${String(err)}`, "err");
  }
}

function importFilters() {
  switch (tab.value) {
    case "mods":
      return [{ name: "Mod", extensions: ["jar", "zip"] }];
    case "resourcepacks":
    case "shaderpacks":
    case "datapacks":
      return [{ name: "压缩包", extensions: ["zip"] }];
    case "schematics":
      return [
        {
          name: "投影文件",
          extensions: ["litematic", "schem", "schematic"],
        },
      ];
    case "screenshots":
      return [{ name: "图片", extensions: ["png", "jpg", "jpeg"] }];
    default:
      return undefined;
  }
}

async function importContent() {
  if (!store.instanceId) return;
  const isSaves = tab.value === "saves";
  const selection = await openDialog({
    multiple: true,
    directory: isSaves,
    title: isSaves ? "选择要导入的存档文件夹" : `选择要导入的${contentLabel.value}`,
    filters: isSaves ? undefined : importFilters(),
  });
  const selected = Array.isArray(selection)
    ? selection
    : selection
      ? [selection]
      : [];
  if (!selected.length) return;
  try {
    const count = await api.instanceImportFiles(
      store.instanceId,
      contentKind.value,
      selected
    );
    await refreshFiles();
    notify(count ? `已导入 ${count} 个文件` : "没有可导入的文件");
  } catch (err) {
    notify(`导入失败：${String(err)}`, "err");
  }
}

async function loadBackups() {
  if (!store.instanceId) return;
  backupsLoading.value = true;
  try {
    backups.value = await api.instanceBackups(store.instanceId);
  } catch (err) {
    backups.value = [];
    notify(`读取存档备份失败：${String(err)}`, "err");
  } finally {
    backupsLoading.value = false;
  }
}

async function backupWorld(file: InstanceFile) {
  if (!store.instanceId || backupBusy.value) return;
  if (store.runningVersionId === store.instanceId) {
    notify("游戏正在运行，请先退出再备份存档", "err");
    return;
  }
  backupBusy.value = file.name;
  try {
    const backup = await api.instanceBackupWorld(store.instanceId, file.name);
    await loadBackups();
    notify(`已备份「${file.display_name}」（${formatBytes(backup.size)}）`);
  } catch (err) {
    notify(`备份存档失败：${String(err)}`, "err");
  } finally {
    backupBusy.value = "";
  }
}

async function importWorldZip() {
  if (!store.instanceId || backupImporting.value) return;
  if (store.runningVersionId === store.instanceId) {
    notify("游戏正在运行，请先退出再导入存档", "err");
    return;
  }
  const path = await pickWorldZip();
  if (!path) return;
  backupImporting.value = true;
  try {
    const name = await api.instanceImportWorld(store.instanceId, path);
    await Promise.all([refreshFiles(), loadBackups()]);
    notify(`已导入存档「${name}」`);
  } catch (err) {
    notify(`导入存档失败：${String(err)}`, "err");
  } finally {
    backupImporting.value = false;
  }
}

async function restoreBackup(backup: WorldBackup) {
  if (!store.instanceId) return;
  if (backupBusy.value) {
    notify("正在处理另一个存档操作，请稍候", "err");
    return;
  }
  if (store.runningVersionId === store.instanceId) {
    notify("游戏正在运行，请先退出再还原存档", "err");
    return;
  }
  const ok = await confirm(
    `还原「${backup.world}」？当前同名存档会先改名为「旧存档 时间戳」保留下来。`,
    { title: "还原存档备份", kind: "warning" }
  );
  if (!ok) return;
  backupBusy.value = backup.path;
  try {
    const name = await api.instanceRestoreBackup(store.instanceId, backup.path);
    await Promise.all([refreshFiles(), loadBackups()]);
    notify(`已还原存档「${name}」`);
  } catch (err) {
    notify(`还原存档失败：${String(err)}`, "err");
  } finally {
    backupBusy.value = "";
  }
}

async function deleteBackup(backup: WorldBackup) {
  if (!store.instanceId) return;
  const ok = await confirm(`删除「${backup.world}」的这份备份？`, {
    title: "删除存档备份",
    kind: "warning",
  });
  if (!ok) return;
  try {
    await api.instanceDeleteBackup(store.instanceId, backup.path);
    await loadBackups();
    notify("备份已删除");
  } catch (err) {
    notify(`删除备份失败：${String(err)}`, "err");
  }
}

async function openBackupsFolder() {
  if (!store.instanceId) return;
  try {
    openFolder(await api.instanceBackupsDir(store.instanceId));
  } catch (err) {
    notify(`打开备份目录失败：${String(err)}`, "err");
  }
}

function openWorldSettings(file: InstanceFile) {
  if (!store.instanceId) return;
  if (store.runningVersionId === store.instanceId) {
    notify("游戏正在运行，请先退出再修改世界设置", "err");
    return;
  }
  worldSettingsFolder.value = file.name;
}

function onWorldSettingsSaved(settings: WorldSettings) {
  notify(`已保存世界设置：${settings.name}`);
  void refreshFiles();
  void loadBackups();
}

function contentIcon() {  switch (tab.value) {
    case "mods":
      return "grid";
    case "resourcepacks":
      return "image";
    case "shaderpacks":
      return "sun";
    case "saves":
      return "database";
    case "screenshots":
      return "folder";
    case "datapacks":
      return "package";
    case "schematics":
      return "layers";
    default:
      return "package";
  }
}

function goTab(next: Tab) {
  tab.value = next;
  if (next === "config" && !configLoading.value) void loadConfigTree();
}

/* ---------- config 文本编辑器 ---------- */

async function loadConfigTree() {
  if (!store.instanceId) return;
  configLoading.value = true;
  configError.value = "";
  try {
    const list = await api.instanceConfigTree(store.instanceId);
    configEntries.value = list;
    // 首次加载默认展开第一层目录；手动刷新时保留用户已展开的状态
    if (!expandedDirs.value.size) {
      expandedDirs.value = new Set(
        list
          .filter((entry) => entry.is_dir && !entry.path.includes("/"))
          .map((entry) => entry.path)
      );
    }
  } catch (err) {
    configEntries.value = [];
    configError.value = String(err);
  } finally {
    configLoading.value = false;
  }
}

async function readConfigFile(path: string) {
  const id = store.instanceId;
  if (!id) return;
  // 从搜索结果点开深层文件时，顺手展开它的父级目录，清空搜索后仍能看到位置
  const parts = path.split("/");
  if (parts.length > 1) {
    const next = new Set(expandedDirs.value);
    for (let i = 1; i < parts.length; i += 1) next.add(parts.slice(0, i).join("/"));
    expandedDirs.value = next;
  }
  try {
    const file = await api.instanceReadText(id, path);
    configPath.value = path;
    configContent.value = file.content;
    configOriginal.value = file.content;
    configEncoding.value = file.encoding;
    configSize.value = file.size;
  } catch (err) {
    notify(`打开失败：${String(err)}`, "err");
  }
}

async function openConfigFile(entry: ConfigEntry) {
  if (entry.is_dir || entry.path === configPath.value) return;
  if (configDirty.value) {
    const ok = await confirm("当前文件有未保存的修改，切换后会丢失。继续吗？", {
      title: "未保存的修改",
      kind: "warning",
    });
    if (!ok) return;
  }
  await readConfigFile(entry.path);
}

async function reloadConfigFile() {
  if (!configPath.value) return;
  if (configDirty.value) {
    const ok = await confirm("重新读取会丢弃当前未保存的修改。继续吗？", {
      title: "重新读取",
      kind: "warning",
    });
    if (!ok) return;
  }
  await readConfigFile(configPath.value);
}

async function saveConfigFile() {
  const id = store.instanceId;
  if (!id || !configPath.value || !configDirty.value || configSaving.value) return;
  configSaving.value = true;
  try {
    await api.instanceWriteText(
      id,
      configPath.value,
      configContent.value,
      configEncoding.value
    );
    configOriginal.value = configContent.value;
    configSize.value = new TextEncoder().encode(configContent.value).length;
    notify("配置已保存，重启游戏后生效");
  } catch (err) {
    notify(`保存失败：${String(err)}`, "err");
  } finally {
    configSaving.value = false;
  }
}

function openConfigFolder() {
  if (paths.value) openFolder(`${paths.value.game_dir}/config`);
}

// 切换实例时清空编辑器状态，避免把上一个实例的内容写进新实例
watch(
  () => store.instanceId,
  () => {
    configEntries.value = [];
    expandedDirs.value = new Set();
    configPath.value = "";
    configContent.value = "";
    configOriginal.value = "";
    configQuery.value = "";
    if (tab.value === "config") void loadConfigTree();
  }
);

/// 汉化资源包入口：CFPA 提供整合包与模组的中文翻译资源
function openTranslationPacks() {
  openExternal("https://cfpa.site/");
}

function downloadMore() {
  if (!info.value) return;  const map: Record<string, string> = {
    mods: "mods",
    resourcepacks: "resourcepacks",
    shaderpacks: "shaders",
    datapacks: "datapacks",
  };
  const kind = map[tab.value];
  if (!kind) return;
  const installKind =
    tab.value === "datapacks" && datapackWorld.value
      ? `datapacks:${datapackWorld.value}`
      : undefined;
  openCommunity(kind, info.value.id, info.value.game_version, info.value.loader, installKind);
}

function findProjectionMod(keyword: string) {
  if (!info.value) return;
  openCommunity(
    "mods",
    info.value.id,
    info.value.game_version,
    info.value.loader,
    undefined,
    keyword
  );
}

async function openExport() {
  if (!info.value || exportBusy.value) return;
  exportBusy.value = true;
  try {
    const preview = await api.modpackExportPreview(info.value.id);
    exportPreview.value = preview;
    exportForm.name = preview.name;
    exportForm.version = preview.version || "1.0.0";
    exportForm.format = "zip";
    exportForm.target = "";
    exportForm.groups = preview.entries
      .filter(
        (entry) =>
          entry.file_count > 0 &&
          !["personal", "saves", "screenshots"].includes(entry.key)
      )
      .map((entry) => entry.key);
    // 建议的额外目录（tacz 枪包等）默认勾选
    exportForm.extras = preview.suggested_extras.map((entry) => entry.key);
    customExtras.value = [];
    customExtraInput.value = "";
    showExport.value = true;
  } catch (err) {
    notify(`读取导出内容失败：${String(err)}`, "err");
  } finally {
    exportBusy.value = false;
  }
}

function toggleExportGroup(key: string) {
  const index = exportForm.groups.indexOf(key);
  if (index >= 0) {
    exportForm.groups.splice(index, 1);
  } else {
    exportForm.groups.push(key);
  }
}

function selectAllGroups() {
  if (!exportPreview.value) return;
  exportForm.groups = exportPreview.value.entries
    .filter((entry) => entry.file_count > 0)
    .map((entry) => entry.key);
}

function clearGroups() {
  exportForm.groups = [];
}

async function pickExportTarget() {
  const extension = exportForm.format === "mrpack" ? "mrpack" : "zip";
  const name = (exportForm.name || "整合包").replace(/[<>:"/\\|?*]/g, "-");
  const path = await savePackDialog(
    `${name}-${exportForm.version || "1.0.0"}.${extension}`,
    exportForm.format
  );
  if (path) exportForm.target = path;
}

async function startExport() {
  if (!info.value || !exportPreview.value) return;
  if (!exportForm.groups.length) {
    notify("至少选择一项导出内容", "err");
    return;
  }
  if (!exportForm.target) {
    notify("请先选择保存位置", "err");
    return;
  }
  const result = await exportModpack(
    info.value.id,
    {
      name: exportForm.name,
      version: exportForm.version,
      format: exportForm.format,
      groups: exportForm.groups,
      target: exportForm.target,
      extra_dirs: [...exportForm.extras, ...customExtras.value],
    },
    exportForm.name
  );
  if (result) {
    showExport.value = false;
    revealFile(result.paths[0] ?? exportForm.target);
  }
}

/// 选择一个新版整合包，先比对再更新
async function openPackUpdate() {
  const value = info.value;
  if (!value || packUpdateBusy.value) return;
  const picked = await openDialog({
    multiple: false,
    filters: [{ name: "整合包", extensions: ["mrpack", "zip"] }],
  });
  const path = typeof picked === "string" ? picked : null;
  if (!path) return;
  packUpdateBusy.value = true;
  try {
    const preview = await api.modpackUpdatePreview(value.id, path);
    packUpdatePath.value = path;
    packUpdatePreview.value = preview;
    packUpdateOpen.value = true;
  } catch (err) {
    notify(`读取新版整合包失败：${String(err)}`, "err");
  } finally {
    packUpdateBusy.value = false;
  }
}

async function applyPackUpdate() {
  const value = info.value;
  if (!value || packUpdating.value || !packUpdatePath.value) return;
  packUpdating.value = true;
  try {
    // 走任务队列：下载面板里能看到进度并取消
    const result = await updateModpack(value.id, packUpdatePath.value);
    packUpdateOpen.value = false;
    if (!result) return;
    await refreshFiles();
    const parts: string[] = [];
    if (result.downloaded) parts.push(`更新 ${result.downloaded} 个文件`);
    if (result.removed) parts.push(`删除 ${result.removed} 个旧文件`);
    if (result.overrides) parts.push(`覆盖 ${result.overrides} 个配置`);
    notify(parts.length ? `整合包已更新：${parts.join("、")}` : "整合包已是最新");
    if (result.failed.length) {
      notify(`有 ${result.failed.length} 个文件失败，可在下载面板重试`, "err");
    }
    if (result.warnings.length) {
      notify(result.warnings[0], "err");
    }
  } catch (err) {
    notify(`更新整合包失败：${String(err)}`, "err");
  } finally {
    packUpdating.value = false;
  }
}

function launch() {
  if (info.value) launchVersion(info.value.id);
}

/// 按钮在运行时显示「停止游戏」，点击要真正执行停止，
/// 否则会走到 launchVersion 里弹出「停止请点停止游戏」的绕圈提示
function launchOrStop() {
  if (store.gameRunning) void stopGame();
  else launch();
}
</script>

<template>
  <div class="page instance">
    <header class="page-head ins-head">
      <IconButton
        icon="chevron-left"
        label="返回版本列表"
        :size="34"
        @click="store.page = 'versions'"
      />
      <span class="ins-tile" :style="{ '--tile': iconColor }">
        <img
          v-if="form.icon === 'custom' && customIcon"
          :src="customIcon"
          class="ins-tile__img"
          alt=""
        />
        <InstanceGlyph v-else :preset="form.icon" :size="28" />
      </span>
      <div class="ins-head__text">
        <h1 class="page-title">{{ info?.display_name ?? "版本设置" }}</h1>
        <p class="page-sub">
          {{ info?.id }}
          <template v-if="info"> · {{ info.game_version }} · {{ kindLabel(info.kind) }} · {{ loaderLabel }}<template v-if="info.loader_version"> {{ info.loader_version }}</template></template>
        </p>
      </div>
      <div class="row ins-head__actions">
        <IconButton
          icon="star"
          :label="form.favorite ? '取消收藏' : '收藏版本'"
          :variant="form.favorite ? 'primary' : 'soft'"
          :size="34"
          @click="toggleFavorite"
        />
        <IconButton
          icon="folder"
          label="打开版本目录"
          variant="soft"
          :size="34"
          @click="openVersionFolder"
        />
        <IconButton
          icon="wrench"
          :label="repairing ? '正在修复…' : '一键修复'"
          variant="soft"
          :size="34"
          :disabled="repairing"
          @click="openRepairOptions"
        />
        <IconButton
          icon="search"
          label="文件自检"
          variant="soft"
          :size="34"
          @click="runSelfCheck"
        />
        <WoodButton
          variant="primary"
          sheen
          @click="launchOrStop"
        >
          <AppIcon :name="store.gameRunning ? 'stop' : 'play'" :size="15" />
          {{ store.gameRunning ? "停止游戏" : "启动游戏" }}
        </WoodButton>
      </div>
    </header>

    <div v-if="loading" class="ins-loading card card-pad">
      <span class="skeleton" />
      <span class="muted small">正在读取版本信息…</span>
    </div>

    <div v-else-if="info" class="ins-layout">
      <aside class="ins-side card">
        <button
          v-for="item in tabs"
          :key="item.key"
          class="ins-side__item"
          :class="{ 'ins-side__item--active': tab === item.key }"
          @click="goTab(item.key)"
        >
          <AppIcon :name="item.icon" :size="16" />
          <span class="ins-side__label">{{ item.label }}</span>
          <span
            v-if="item.key === 'mods' || item.key === 'resourcepacks' || item.key === 'shaderpacks' || item.key === 'saves' || item.key === 'screenshots'"
            class="ins-side__count mono"
          >
            {{ item.key === "mods" ? info.counts.mods : item.key === "resourcepacks" ? info.counts.resourcepacks : item.key === "shaderpacks" ? info.counts.shaderpacks : item.key === "saves" ? info.counts.saves : info.counts.screenshots }}
          </span>
        </button>

        <div class="ins-side__meta">
          <span class="ins-side__key">版本目录</span>
          <span class="ins-side__path mono">{{ info.path }}</span>
          <span v-if="info.favorite" class="tag tag--grass ins-side__fav">已收藏</span>
        </div>
      </aside>

      <div class="ins-main">
        <!-- 概览 -->
        <template v-if="tab === 'overview'">
          <section class="card card-pad group reveal">
            <div class="group-head">
              <div class="group-titles">
                <div class="section-title">个性化</div>
              </div>
              <span class="group-line" />
            </div>

            <div class="setting">
              <div class="setting__label">
                <strong>显示名称</strong>
                <span>只影响启动器显示，不改内部版本 ID</span>
              </div>
              <div class="setting__control">
                <WoodInput v-model="form.display_name" :placeholder="info.id" />
              </div>
            </div>

            <div class="setting">
              <div class="setting__label">
                <strong>分类</strong>
                <span>用来在版本列表里区分用途</span>
              </div>
              <div class="setting__control">
                <WoodSelect v-model="form.category" :options="categoryOptions" />
              </div>
            </div>

            <div class="setting">
              <div class="setting__label">
                <strong>图标</strong>
                <span>版本列表与首页使用同一个图标</span>
              </div>
              <div class="setting__control setting__control--wide">
                <div class="icon-grid">
                  <button
                    v-for="preset in iconPresets"
                    :key="preset.value"
                    class="icon-chip"
                    :class="{ 'icon-chip--active': form.icon === preset.value }"
                    @click="form.icon = preset.value"
                  >
                    <span
                      class="icon-chip__tile"
                      :style="{ '--tile': preset.color }"
                    >
                      <InstanceGlyph :preset="preset.value" :size="19" />
                    </span>
                    {{ preset.label }}
                  </button>
                  <button
                    class="icon-chip"
                    :class="{ 'icon-chip--active': form.icon === 'custom' }"
                    @click="pickIconImage"
                  >
                    <span class="icon-chip__tile" :style="{ '--tile': '#8b9d6f' }">
                      <img
                        v-if="customIcon"
                        :src="customIcon"
                        class="icon-chip__img"
                        alt=""
                      />
                      <AppIcon v-else name="image" :size="18" />
                    </span>
                    自定义
                  </button>
                </div>
              </div>
            </div>

            <div class="setting setting--last">
              <div class="setting__label">
                <strong>描述</strong>
                <span>写点这个版本是做什么的，方便以后辨认</span>
              </div>
              <div class="setting__control setting__control--wide">
                <textarea
                  v-model="form.description"
                  class="ins-textarea"
                  rows="3"
                  placeholder="例如：1.21.1 生存整合包，带优化 Mod"
                />
              </div>
            </div>
          </section>

          <section class="card card-pad group reveal" style="--i: 1">
            <div class="group-head">
              <div class="group-titles">
                <div class="section-title">快捷方式</div>
              </div>
              <span class="group-line" />
            </div>

            <div class="quick-grid">
              <WoodButton variant="soft" @click="openVersionFolder">
                <AppIcon name="folder" :size="15" />
                版本目录
              </WoodButton>
              <WoodButton variant="soft" @click="openGameFolder">
                <AppIcon name="folder" :size="15" />
                游戏目录
              </WoodButton>
              <WoodButton variant="soft" @click="goTab('mods')">
                <AppIcon name="grid" :size="15" />
                Mod 文件夹
              </WoodButton>
              <WoodButton variant="soft" @click="goTab('saves')">
                <AppIcon name="database" :size="15" />
                存档文件夹
              </WoodButton>
              <WoodButton variant="soft" @click="goTab('screenshots')">
                <AppIcon name="folder" :size="15" />
                截图文件夹
              </WoodButton>
              <WoodButton variant="soft" :disabled="exportBusy || packUpdateBusy" @click="openExport">
                <AppIcon name="download" :size="15" />
                导出整合包
              </WoodButton>
              <WoodButton variant="soft" :disabled="packUpdateBusy" @click="openPackUpdate">
                <AppIcon name="refresh" :size="15" />
                {{ packUpdateBusy ? "读取中…" : "更新整合包" }}
              </WoodButton>
              <WoodButton variant="soft" @click="openCrashDialog(info.id)">
                <AppIcon name="alert" :size="15" />
                崩溃分析
              </WoodButton>
            </div>
          </section>

          <section class="card card-pad group reveal" style="--i: 2">
            <div class="group-head">
              <div class="group-titles">
                <div class="section-title">详细信息</div>
              </div>
              <span class="group-line" />
            </div>

            <div class="detail-grid">
              <div class="detail">
                <span class="detail__label">游戏版本</span>
                <span class="detail__value">{{ info.game_version }}</span>
              </div>
              <div class="detail">
                <span class="detail__label">加载器</span>
                <span class="detail__value detail__value--row">
                  {{ loaderLabel }}
                  <template v-if="info.loader_version">{{ info.loader_version }}</template>
                  <button
                    v-if="canReinstallLoader"
                    class="detail__action"
                    :disabled="updatingLoader"
                    @click="openLoaderUpdate"
                  >
                    更新
                  </button>
                </span>
              </div>
              <div class="detail">
                <span class="detail__label">类型</span>
                <span class="detail__value">{{ kindLabel(info.kind) }}</span>
              </div>
              <div class="detail">
                <span class="detail__label">所需 Java</span>
                <span class="detail__value">Java {{ info.java_major }}</span>
              </div>
              <div class="detail">
                <span class="detail__label">安装于</span>
                <span class="detail__value">{{ formatDate(new Date(info.installed_at * 1000).toISOString()) }}</span>
              </div>
              <div class="detail">
                <span class="detail__label">目录模式</span>
                <span class="detail__value">{{ store.config?.version_isolation ? "版本隔离" : "共用目录" }}</span>
              </div>
            </div>

            <div class="count-strip">
              <button class="count" @click="goTab('mods')">
                <span class="count__num display">{{ info.counts.mods }}</span>
                <span class="count__label">Mod</span>
              </button>
              <button class="count" @click="goTab('resourcepacks')">
                <span class="count__num display">{{ info.counts.resourcepacks }}</span>
                <span class="count__label">资源包</span>
              </button>
              <button class="count" @click="goTab('shaderpacks')">
                <span class="count__num display">{{ info.counts.shaderpacks }}</span>
                <span class="count__label">光影</span>
              </button>
              <button class="count" @click="goTab('saves')">
                <span class="count__num display">{{ info.counts.saves }}</span>
                <span class="count__label">存档</span>
              </button>
              <button class="count" @click="goTab('screenshots')">
                <span class="count__num display">{{ info.counts.screenshots }}</span>
                <span class="count__label">截图</span>
              </button>
            </div>
          </section>
        </template>

        <!-- 启动设置 -->
        <section v-else-if="tab === 'launch'" class="card card-pad group reveal">
          <div class="group-head">
            <span class="group-no display">02</span>
            <div class="group-titles">
              <div class="section-title">启动设置</div>
            </div>
            <span class="group-line" />
          </div>

          <p class="muted small">
            未单独设置的项默认跟随全局；这里改的值只对当前版本生效，下次启动时使用。
          </p>

          <div class="setting">
            <div class="setting__label">
              <strong>最大内存</strong>
              <span>
                {{ form.memory ? `${form.memory} MB` : `跟随全局 · ${store.config?.memory_auto ? `自动约 ${(recommendedMemory / 1024).toFixed(1)} GB` : `${store.config?.memory_mb ?? 4096} MB`}` }}
              </span>
            </div>
            <div class="setting__control">
              <WoodInput v-model="form.memory" type="number" placeholder="例如 6144" />
            </div>
          </div>

          <div class="setting">
            <div class="setting__label">
              <strong>内存管理</strong>
              <span>默认跟随全局；选「自定义」时不追加任何 GC 参数</span>
            </div>
            <div class="setting__control">
              <WoodSelect v-model="form.memoryMode" :options="memoryModeOptions" />
            </div>
          </div>

          <div class="setting">
            <div class="setting__label">
              <strong>Java 运行时</strong>
              <span>该版本独用的 Java；默认跟随全局设置</span>
            </div>
            <div class="setting__control setting__control--wide">
              <WoodSelect v-model="form.javaPath" :options="javaOptions" />
            </div>
          </div>

          <div class="setting setting--stack">
            <div class="setting__label">
              <strong>JVM 参数</strong>
              <span>追加在全局参数之后，空格分隔</span>
            </div>
            <div class="setting__control setting__control--wide">
              <WoodInput v-model="form.javaArgs" mono placeholder="例如 -XX:+UseZGC" />
            </div>
          </div>

          <div class="setting setting--stack">
            <div class="setting__label">
              <strong>游戏参数</strong>
              <span>追加到游戏主类之后，例如 --width 1600 --height 900</span>
            </div>
            <div class="setting__control setting__control--wide">
              <WoodInput v-model="form.gameArgs" mono placeholder="--width 1600 --height 900" />
            </div>
          </div>

          <div class="setting setting--stack">
            <div class="setting__label">
              <strong>启动前执行命令</strong>
              <span>工作目录为版本游戏目录，输出会写入启动日志</span>
            </div>
            <div class="setting__control setting__control--wide">
              <WoodInput v-model="form.preLaunch" mono placeholder="例如 java -jar packwiz-installer-bootstrap.jar ..." />
            </div>
          </div>

          <div class="setting">
            <div class="setting__label">
              <strong>Java Launch Wrapper</strong>
              <span>禁用后不再注入 DOCL 启动器标识，部分反作弊需要</span>
            </div>
            <div class="setting__control">
              <WoodSelect v-model="form.wrapper" :options="wrapperOptions" />
            </div>
          </div>

          <div class="setting">
            <div class="setting__label">
              <strong>LWJGL Unsafe 兼容</strong>
              <span>Java 24+ 放行 Unsafe 内存访问，避免 LWJGL 告警</span>
            </div>
            <div class="setting__control">
              <WoodSelect v-model="form.unsafe" :options="unsafeOptions" />
            </div>
          </div>

          <div class="setting">
            <div class="setting__label">
              <strong>高性能显卡</strong>
              <span>为当前 Java 写入 Windows 显卡偏好</span>
            </div>
            <div class="setting__control">
              <WoodSelect v-model="form.gpu" :options="gpuOptions" />
            </div>
          </div>

          <div class="setting">
            <div class="setting__label">
              <strong>窗口尺寸</strong>
              <span>默认使用全局分辨率</span>
            </div>
            <div class="setting__control">
              <div class="row resolution">
                <WoodInput v-model="form.width" type="number" placeholder="宽" />
                <span class="muted resolution__x">×</span>
                <WoodInput v-model="form.height" type="number" placeholder="高" />
              </div>
            </div>
          </div>

          <div class="setting setting--last">
            <div class="setting__label">
              <strong>全屏</strong>
              <span>启动时是否直接进入全屏</span>
            </div>
            <div class="setting__control">
              <WoodSelect v-model="form.fullscreen" :options="fullscreenOptions" />
            </div>
          </div>
        </section>

        <!-- 游戏设置预设 -->
        <section v-else-if="tab === 'gameopts'" class="card card-pad group reveal">
          <div class="group-head">
            <span class="group-no display">10</span>
            <div class="group-titles">
              <div class="section-title">游戏设置预设</div>
            </div>
            <span class="group-line" />
          </div>

          <p class="muted small">
            把当前版本的 options.txt（视野、视频、按键、音量、资源包列表）保存成模板，之后手动应用到任意版本。
            模组自己的 config 不在其中；应用前请先退出游戏，否则游戏退出时会重写 options.txt。
          </p>

          <div class="setting setting--stack">
            <div class="setting__label">
              <strong>从当前版本保存</strong>
              <span>先在游戏里调好设置并退出，再保存；同名预设会覆盖</span>
            </div>
            <div class="setting__control setting__control--wide">
              <div class="row opts-save">
                <WoodInput v-model="optionsName" placeholder="预设名称，例如 低配流畅" />
                <WoodButton
                  variant="soft"
                  size="sm"
                  :disabled="optionsBusy"
                  @click="saveOptionsPreset"
                >
                  <AppIcon name="download" :size="14" />
                  保存当前设置
                </WoodButton>
              </div>
            </div>
          </div>

          <div class="divider" />

          <div class="opts-presets">
            <div class="opts-presets__head">
              <strong>已保存的预设</strong>
              <span class="muted small">应用会先把当前 options.txt 备份为 options.txt.bak</span>
            </div>
            <div v-if="!optionsPresets.length" class="muted small">
              还没有预设。先按上面的方式保存一套当前设置。
            </div>
            <div v-for="preset in optionsPresets" :key="preset.id" class="opts-preset">
              <div class="opts-preset__info">
                <strong>{{ preset.name }}</strong>
                <span class="muted small mono">
                  {{ presetDate(preset) }} · {{ formatBytes(preset.size) }}
                </span>
              </div>
              <div class="row">
                <WoodButton
                  variant="primary"
                  size="sm"
                  :disabled="optionsBusy"
                  @click="applyOptionsPreset(preset)"
                >
                  应用
                </WoodButton>
                <IconButton
                  icon="trash"
                  label="删除预设"
                  variant="danger-quiet"
                  :size="30"
                  @click="deleteOptionsPreset(preset)"
                />
              </div>
            </div>
          </div>
        </section>

        <!-- 配置文件 -->
        <section v-else-if="tab === 'config'" class="card card-pad group reveal">
          <div class="group-head">
            <div class="group-titles">
              <div class="section-title">配置文件</div>
              <p class="muted small">
                直接编辑 config 下的文本文件；游戏运行时修改可能在退出时被覆盖，建议先退出游戏。Ctrl + S 保存。
              </p>
            </div>
            <span class="group-line" />
            <div class="row">
              <IconButton
                icon="refresh"
                label="重新读取目录"
                variant="soft"
                :size="32"
                @click="loadConfigTree"
              />
              <WoodButton variant="ghost" size="sm" @click="openConfigFolder">
                <AppIcon name="folder" :size="14" />
                config 目录
              </WoodButton>
            </div>
          </div>

          <div class="cfg">
            <div class="cfg__side">
              <WoodInput v-model="configQuery" placeholder="搜索文件，例如 jei" />
              <div class="cfg__list">
                <div v-if="configLoading" class="loading-block">
                  <span class="loading-block__bar" />
                  正在读取…
                </div>
                <div v-else-if="configError" class="state state--error state--inline">
                  <span class="state__icon"><AppIcon name="alert" :size="16" /></span>
                  <div class="state__text">
                    <p class="state__title">读取失败</p>
                    <p class="state__hint">{{ configError }}</p>
                    <div class="state__actions">
                      <WoodButton variant="soft" size="sm" @click="loadConfigTree">重试</WoodButton>
                    </div>
                  </div>
                </div>
                <div v-else-if="!configEntries.length" class="state state--inline">
                  <span class="state__icon"><AppIcon name="folder" :size="16" /></span>
                  <div class="state__text">
                    <p class="state__title">没有可编辑的文件</p>
                    <p class="state__hint">先启动一次游戏生成配置。</p>
                  </div>
                </div>
                <div v-else-if="!visibleConfigEntries.length" class="state state--inline">
                  <span class="state__icon"><AppIcon name="search" :size="16" /></span>
                  <div class="state__text">
                    <p class="state__title">没有匹配的文件</p>
                    <p class="state__hint">换个关键词试试。</p>
                  </div>
                </div>
                <template v-else>
                  <template v-for="entry in visibleConfigEntries" :key="entry.path">
                    <button
                      v-if="entry.is_dir"
                      class="cfg__dir"
                      :style="{
                        paddingLeft: `${8 + (configSearching ? 0 : configDepth(entry.path)) * 12}px`,
                      }"
                      @click="toggleDir(entry.path)"
                    >
                      <AppIcon
                        name="chevron-down"
                        :size="11"
                        class="cfg__chev"
                        :class="{ 'cfg__chev--collapsed': !isDirExpanded(entry.path) }"
                      />
                      <AppIcon name="folder" :size="12" />
                      <span class="cfg__name">{{ entry.name }}</span>
                    </button>
                    <button
                      v-else
                      class="cfg__file"
                      :class="{ 'cfg__file--active': entry.path === configPath }"
                      :style="{
                        paddingLeft: `${
                          10 + (configSearching ? 0 : configDepth(entry.path)) * 12
                        }px`,
                      }"
                      @click="openConfigFile(entry)"
                    >
                      <AppIcon name="list" :size="12" />
                      <span class="cfg__name">{{ entry.name }}</span>
                      <span
                        v-if="entry.path === configPath && configDirty"
                        class="cfg__dot"
                        title="有未保存的修改"
                      />
                    </button>
                  </template>
                </template>
              </div>
            </div>

            <div class="cfg__main">
              <div class="cfg__bar">
                <span class="mono cfg__path">
                  {{ configPath || "从左侧选择一个配置文件" }}
                </span>
                <span v-if="configPath" class="muted small cfg__meta">
                  {{ configEncoding === "gbk" ? "GBK" : "UTF-8" }} · {{ formatBytes(configSize) }}
                </span>
                <div class="row cfg__actions">
                  <WoodButton
                    v-if="configPath && !plainConfigEditor"
                    variant="ghost"
                    size="sm"
                    title="在文件内容中查找（Ctrl + F）"
                    @click="configEditorRef?.openSearch()"
                  >
                    <AppIcon name="search" :size="13" />
                    查找
                  </WoodButton>
                  <WoodButton
                    variant="ghost"
                    size="sm"
                    :disabled="!configPath || configLoading"
                    @click="reloadConfigFile"
                  >
                    重新读取
                  </WoodButton>
                  <WoodButton
                    variant="primary"
                    size="sm"
                    :disabled="!configDirty || configSaving"
                    @click="saveConfigFile"
                  >
                    {{ configSaving ? "保存中…" : "保存" }}
                  </WoodButton>
                </div>
              </div>
              <ConfigCodeEditor
                v-if="configPath && !plainConfigEditor"
                :key="configPath"
                ref="configEditorRef"
                v-model="configContent"
                :path="configPath"
                :disabled="configSaving"
                @save="saveConfigFile"
              />
              <textarea
                v-else
                v-model="configContent"
                class="cfg__editor"
                :class="{ 'cfg__editor--empty': !configPath }"
                :disabled="!configPath"
                spellcheck="false"
                :placeholder="configPath ? '' : '选择左侧文件后在这里编辑'"
                @keydown.ctrl.s.prevent="saveConfigFile"
              />
            </div>
          </div>
        </section>

        <!-- 内容管理 -->
        <section v-else-if="isContentTab" class="card card-pad group reveal">
          <div class="group-head">
            <span class="group-no display">03</span>
            <div class="group-titles">
              <div class="section-title">{{ contentLabel }}</div>
            </div>
            <span class="group-line" />
            <div class="row">
              <WoodButton
                v-if="tab === 'mods'"
                variant="soft"
                size="sm"
                :disabled="updatesLoading"
                @click="checkModUpdates"
              >
                <AppIcon name="refresh" :size="14" />
                {{ updatesLoading ? "检查中…" : "检查更新" }}
              </WoodButton>
              <WoodButton
                v-if="canDownloadMore && !noWorlds"
                variant="primary"
                size="sm"
                sheen
                @click="downloadMore"
              >
                <AppIcon name="download" :size="14" />
                下载新{{ contentLabel }}
              </WoodButton>
              <WoodButton
                v-if="tab === 'resourcepacks'"
                variant="ghost"
                size="sm"
                title="打开 CFPA 汉化资源包页面，安装后可汉化整合包与模组"
                @click="openTranslationPacks"
              >
                <AppIcon name="external" :size="13" />
                汉化资源包
              </WoodButton>
              <WoodButton variant="ghost" size="sm" @click="openContentFolder">
                <AppIcon name="folder" :size="14" />
                打开文件夹
              </WoodButton>
              <WoodButton
                v-if="tab === 'saves'"
                variant="soft"
                size="sm"
                :disabled="backupImporting"
                @click="importWorldZip"
              >
                <AppIcon name="package" :size="14" />
                {{ backupImporting ? "导入中…" : "导入压缩包" }}
              </WoodButton>
              <WoodButton v-if="!noWorlds" variant="soft" size="sm" @click="importContent">
                <AppIcon name="plus" :size="14" />
                {{ tab === "saves" ? "导入存档文件夹" : "从文件导入" }}
              </WoodButton>
              <IconButton
                icon="refresh"
                label="刷新列表"
                variant="soft"
                :size="32"
                @click="refreshFiles"
              />
            </div>
          </div>

          <p class="muted small">{{ contentHint }}</p>

          <div v-if="tab === 'schematics'" class="proj-tip">
            <span class="proj-tip__icon">
              <AppIcon name="layers" :size="17" />
            </span>
            <div class="proj-tip__main">
              <strong>投影助手</strong>
              <span class="muted small">
                投影文件放进 schematics 文件夹后，在游戏内用 Litematica 或 WorldEdit
                载入并放置。缺 Mod 时可以先从资源站安装。
              </span>
              <span
                v-if="projectionStatus"
                class="proj-tip__status"
                :class="{ 'proj-tip__status--on': projectionMods?.litematica || projectionMods?.worldedit }"
              >
                <AppIcon
                  :name="projectionMods?.litematica || projectionMods?.worldedit ? 'check' : 'alert'"
                  :size="12"
                />
                {{ projectionStatus }}
              </span>
            </div>
            <div class="row">
              <WoodButton variant="soft" size="sm" @click="findProjectionMod('litematica')">
                <AppIcon name="download" :size="13" />
                Litematica
              </WoodButton>
              <WoodButton variant="ghost" size="sm" @click="findProjectionMod('worldedit')">
                WorldEdit
              </WoodButton>
            </div>
          </div>

          <div v-if="tab === 'datapacks'" class="world-pick">
            <span class="world-pick__label">目标世界</span>
            <WoodSelect
              v-if="worlds.length"
              :model-value="datapackWorld"
              :options="worldOptions"
              @update:model-value="switchWorld"
            />
            <span v-else class="muted small">
              这个版本还没有存档世界，数据包需要放进具体世界。
            </span>
          </div>

          <div v-if="tab === 'saves'" class="save-backups">
            <header class="save-backups__head">
              <div class="save-backups__titles">
                <strong>存档备份</strong>
                <span class="muted small">
                  备份放在实例的 backups 目录；还原时同名存档会先改名为旧存档保留
                </span>
              </div>
              <div class="row">
                <WoodButton
                  variant="ghost"
                  size="sm"
                  :disabled="backupsLoading"
                  @click="loadBackups"
                >
                  <AppIcon name="refresh" :size="13" />
                  刷新备份
                </WoodButton>
                <WoodButton variant="ghost" size="sm" @click="openBackupsFolder">
                  <AppIcon name="folder" :size="13" />
                  备份目录
                </WoodButton>
              </div>
            </header>

            <div v-if="backupsLoading" class="file-state">
              <span class="skeleton" />
              <span class="muted small">正在读取备份…</span>
            </div>
            <p v-else-if="!backups.length" class="save-backups__empty muted small">
              还没有备份。点存档右侧的下载按钮，就能为这个存档生成一份 zip。
            </p>
            <div v-else class="save-backups__list">
              <div v-for="backup in backups" :key="backup.path" class="backup-row">
                <span class="backup-row__icon">
                  <AppIcon name="database" :size="15" />
                </span>
                <div class="backup-row__main">
                  <strong :title="backup.world">{{ backup.world }}</strong>
                  <span class="muted small mono">
                    {{ formatDate(new Date(backup.created * 1000).toISOString()) }} ·
                    {{ formatBytes(backup.size) }}
                  </span>
                </div>
                <WoodButton
                  variant="soft"
                  size="sm"
                  :disabled="backupBusy === backup.path"
                  @click="restoreBackup(backup)"
                >
                  {{ backupBusy === backup.path ? "还原中…" : "还原" }}
                </WoodButton>
                <IconButton
                  icon="trash"
                  label="删除备份"
                  variant="danger-quiet"
                  :size="28"
                  @click="deleteBackup(backup)"
                />
              </div>
            </div>
          </div>

          <div v-if="tab === 'mods' && updatesLoading" class="update-state">
            <span class="skeleton" />
            正在对照 Modrinth 检查更新…
          </div>
          <div
            v-else-if="tab === 'mods' && updateError"
            class="update-state update-state--err"
          >
            <AppIcon name="alert" :size="14" />
            {{ updateError }}
            <WoodButton variant="ghost" size="sm" @click="checkModUpdates">
              重试
            </WoodButton>
          </div>
          <div v-else-if="tab === 'mods' && modUpdates" class="update-panel">
            <header class="update-panel__head">
              <div class="update-panel__title">
                <strong>
                  {{
                    modUpdates.updates.length
                      ? `${modUpdates.updates.length} 个 Mod 可更新`
                      : "所有 Mod 都是最新的"
                  }}
                </strong>
                <span class="muted small">
                  扫描 {{ modUpdates.scanned }} 个 · 识别 {{ modUpdates.matched }} 个 · 已是最新
                  {{ modUpdates.up_to_date }} 个
                </span>
              </div>
              <div class="row">
                <WoodButton
                  v-if="modUpdates.updates.length"
                  variant="primary"
                  size="sm"
                  :disabled="!!updatingKey"
                  @click="applyAllModUpdates"
                >
                  <AppIcon name="download" :size="13" />
                  {{ updatingKey ? "更新中…" : "全部更新" }}
                </WoodButton>
                <IconButton
                  icon="close"
                  label="关闭更新结果"
                  :size="28"
                  @click="modUpdates = null"
                />
              </div>
            </header>

            <div v-if="modUpdates.updates.length" class="update-list">
              <div
                v-for="item in modUpdates.updates"
                :key="item.file_name"
                class="update-row"
              >
                <span class="update-row__icon">
                  <AppIcon name="package" :size="16" />
                </span>
                <div class="update-row__main">
                  <strong>
                    {{ item.mod_name }}
                    <button
                      v-if="item.project_url"
                      class="update-row__link"
                      title="打开项目页"
                      @click="openExternal(item.project_url)"
                    >
                      <AppIcon name="external" :size="11" />
                    </button>
                  </strong>
                  <span class="muted small mono">
                    {{ item.current_version || "未知版本" }} → {{ item.latest_version }}
                  </span>
                  <span class="update-row__file mono">{{ item.latest_file }}</span>
                </div>
                <span class="tag">
                  {{ item.source === "curseforge" ? "CF" : "MR" }}
                </span>
                <span v-if="item.version_type !== 'release'" class="tag">
                  {{ item.version_type }}
                </span>
                <WoodButton
                  variant="primary"
                  size="sm"
                  :disabled="!!updatingKey"
                  @click="applyModUpdate(item)"
                >
                  {{ updatingKey === item.file_name ? "更新中…" : "更新" }}
                </WoodButton>
              </div>
            </div>
            <ul v-else-if="modUpdates.notes.length" class="update-notes">
              <li v-for="(note, index) in modUpdates.notes" :key="index">
                {{ note }}
              </li>
            </ul>
          </div>

          <div v-if="files.length" class="file-search">
            <WoodInput
              v-model="fileQuery"
              :placeholder="`搜索${contentLabel}文件名`"
            />
            <span class="muted small">{{ filteredFiles.length }} / {{ files.length }}</span>
          </div>

          <div v-if="filesLoading" class="file-state">
            <span class="skeleton" />
            <span class="muted small">正在读取…</span>
          </div>
          <div v-else-if="!files.length" class="file-empty">
            <AppIcon :name="contentIcon()" :size="28" />
            <div>
              <p class="file-empty__title">还没有{{ contentLabel }}</p>
              <p class="muted small">
                {{
                  noWorlds
                    ? "先在游戏里创建一个世界，数据包会安装到该世界的 datapacks 文件夹。"
                    : tab === "screenshots"
                      ? "进游戏按 F2 截图，图片就会出现在这里。"
                      : tab === "saves"
                        ? "在游戏里新建世界，或点右上角导入压缩包 / 存档文件夹。"
                        : tab === "schematics"
                          ? "把 .litematic / .schem / .schematic 投影文件放进这里，游戏内用 Mod 载入。"
                          : `点击「下载新${contentLabel}」去资源站挑选，或者从本地文件导入。`
                }}
              </p>
            </div>
          </div>
          <div v-else-if="!filteredFiles.length" class="file-state">
            <span class="muted small">没有匹配的文件</span>
          </div>
          <div v-else-if="tab === 'screenshots'" class="shot-grid">
            <article v-for="file in visibleFiles" :key="file.name" class="shot-card">
              <button class="shot-card__thumb" @click="openShot(file)">
                <img
                  v-if="thumbOf(file)"
                  :src="thumbOf(file)"
                  :alt="file.display_name"
                  loading="lazy"
                />
                <span v-else class="shot-card__blank">
                  <AppIcon name="image" :size="22" />
                </span>
              </button>
              <footer class="shot-card__foot">
                <span class="shot-card__name" :title="file.display_name">
                  {{ file.display_name }}
                </span>
                <IconButton
                  icon="trash"
                  :label="confirmName === file.name ? '确认删除' : '删除'"
                  :variant="confirmName === file.name ? 'danger' : 'danger-quiet'"
                  :size="26"
                  @click="requestDelete(file)"
                />
              </footer>
            </article>
            <div v-if="hiddenFileCount" class="file-more">
              <WoodButton variant="ghost" size="sm" @click="showMoreFiles">
                显示更多（还有 {{ hiddenFileCount }} 个）
              </WoodButton>
            </div>
          </div>
          <div v-else class="file-list">
            <div v-for="file in visibleFiles" :key="file.name" class="file-row">
              <span
                v-if="modMetaOf(file)"
                class="file-row__thumb file-row__thumb--mod"
                :title="modMetaOf(file)?.name ?? ''"
              >
                <img
                  v-if="modMetaOf(file)?.logo_url"
                  :src="modMetaOf(file)?.logo_url"
                  alt=""
                  loading="lazy"
                />
                <AppIcon v-else name="package" :size="16" />
              </span>
              <span v-else-if="thumbOf(file)" class="file-row__thumb">
                <img :src="thumbOf(file)" alt="" loading="lazy" />
              </span>
              <span v-else class="file-row__icon">
                <AppIcon :name="contentIcon()" :size="16" />
              </span>
              <div class="file-row__main">
                <span class="file-row__name">
                  {{ file.display_name }}
                  <span v-if="!file.enabled" class="tag file-row__off">已禁用</span>
                </span>
                <span class="file-row__meta mono">
                  {{ file.is_dir && !file.size ? "文件夹" : formatBytes(file.size) }}
                  <template v-if="modMetaOf(file)">
                    · {{ modMetaOf(file)?.source === "modrinth" ? "Modrinth" : "CurseForge" }}
                    <template v-if="modMetaOf(file)?.version_number">
                      {{ modMetaOf(file)?.version_number }}
                    </template>
                  </template>
                  · {{ formatDate(new Date(file.modified * 1000).toISOString()) }}
                </span>
              </div>
              <IconButton
                v-if="modMetaOf(file)?.website_url"
                icon="external"
                label="打开来源页面"
                variant="ghost"
                :size="30"
                @click="openModSource(file)"
              />
              <WoodToggle
                v-if="tab === 'mods' || tab === 'resourcepacks' || tab === 'shaderpacks'"
                class="file-toggle"
                :model-value="file.enabled"
                :disabled="togglingName === file.name"
                :title="file.enabled ? '禁用' : '启用'"
                @update:model-value="toggleFile(file)"
              />
              <IconButton
                v-if="tab === 'saves' && file.is_dir"
                icon="settings"
                label="世界设置"
                variant="ghost"
                :size="30"
                @click="openWorldSettings(file)"
              />
              <IconButton
                v-if="tab === 'saves' && file.is_dir"
                icon="download"
                :label="backupBusy === file.name ? '正在备份…' : '备份此存档'"
                variant="ghost"
                :size="30"
                :disabled="backupBusy === file.name"
                @click="backupWorld(file)"
              />
              <IconButton
                v-if="tab === 'saves' && file.is_dir"
                icon="globe"
                label="种子预览"
                variant="ghost"
                :size="30"
                @click="openSeed(file)"
              />
              <IconButton
                icon="trash"
                :label="confirmName === file.name ? '确认删除' : '删除'"
                :variant="confirmName === file.name ? 'danger' : 'danger-quiet'"
                :size="30"
                @click="requestDelete(file)"
              />
            </div>
            <div v-if="hiddenFileCount" class="file-more">
              <WoodButton variant="ghost" size="sm" @click="showMoreFiles">
                显示更多（还有 {{ hiddenFileCount }} 个）
              </WoodButton>
            </div>
          </div>
        </section>
      </div>
    </div>

    <WoodModal v-if="seedOpen" title="种子预览" :width="430" @close="seedOpen = false">
      <div v-if="seedBusy" class="seed-loading">
        <span class="skeleton" />
        <span class="muted small">正在读取 level.dat…</span>
      </div>
      <div v-else-if="seedInfo" class="seed-body">
        <div class="seed-world">
          <strong>{{ seedInfo.name }}</strong>
          <span class="muted small mono">
            {{ seedInfo.folder }}<template v-if="seedInfo.version"> · {{ seedInfo.version }}</template>
          </span>
        </div>
        <div class="seed-value">
          <span v-if="seedInfo.seed" class="seed-value__text mono">{{ seedInfo.seed }}</span>
          <span v-else class="muted small">这个存档的 level.dat 里没有读到种子。</span>
        </div>
        <p class="muted small seed-hint">
          种子地图会在浏览器中打开，进入后选择对应的游戏版本即可查看生物群系、结构与要塞位置。
        </p>
      </div>
      <template #footer>
        <WoodButton variant="ghost" size="sm" @click="seedOpen = false">关闭</WoodButton>
        <WoodButton variant="soft" size="sm" :disabled="!seedInfo?.seed" @click="copySeed">
          <AppIcon name="copy" :size="13" />
          复制种子
        </WoodButton>
        <WoodButton variant="primary" size="sm" :disabled="!seedInfo?.seed" @click="openSeedMap">
          <AppIcon name="external" :size="13" />
          打开种子地图
        </WoodButton>
      </template>
    </WoodModal>

    <WorldSettingsModal
      v-if="worldSettingsFolder && store.instanceId"
      :version-id="store.instanceId"
      :folder="worldSettingsFolder"
      @close="worldSettingsFolder = null"
      @saved="onWorldSettingsSaved"
    />

    <WoodModal
      v-if="checkOpen"
      title="实例文件自检"
      :width="470"
      @close="checkOpen = false"
    >
      <div v-if="checking" class="check-loading">
        <span class="skeleton" />
        <span class="muted small">正在检查 Mod / 资源包 / 光影…</span>
      </div>
      <div v-else-if="checkResult" class="check-body">
        <p class="muted small">已检查 {{ checkResult.scanned }} 个文件。</p>
        <p v-if="checkResult.broken.length === 0" class="check-ok">
          <AppIcon name="check" :size="14" />
          没有发现损坏的文件
        </p>
        <template v-else>
          <ul class="check-list">
            <li
              v-for="file in checkResult.broken"
              :key="`${file.kind}/${file.name}`"
              class="check-item"
            >
              <span class="check-tag">{{ CHECK_KIND_LABELS[file.kind] ?? file.kind }}</span>
              <span class="check-name mono">{{ file.name }}</span>
              <span class="muted small">{{ formatBytes(file.size) }}</span>
            </li>
          </ul>
          <p class="muted small">
            这些文件已经无法作为压缩包读取，通常是下载中断造成的。删除后重新下载即可，否则游戏会在 Mod 扫描阶段直接退出。
          </p>
        </template>
      </div>
      <template #footer>
        <WoodButton variant="ghost" size="sm" @click="checkOpen = false">关闭</WoodButton>
        <WoodButton
          v-if="checkResult?.broken.length"
          variant="soft"
          size="sm"
          @click="openBrokenFolder"
        >
          <AppIcon name="folder" :size="13" />
          打开所在目录
        </WoodButton>
      </template>
    </WoodModal>

    <WoodModal
      v-if="repairOptionsOpen"
      title="一键修复"
      :width="470"
      @close="repairOptionsOpen = false"
    >
      <div class="check-body">
        <p class="muted small">
          会修版本清单、补回缺失文件、重解本地库，并禁用重复或不兼容的模组。
          下面几项需要联网下载或修改设置，按需勾选：
        </p>
        <label class="repair-option">
          <input v-model="repairOptions.install_deps" type="checkbox" />
          <span>
            <strong>自动补装缺失前置</strong>
            <em>从 Modrinth 下载缺少的前置 Mod，可能涉及多个文件</em>
          </span>
        </label>
        <label class="repair-option">
          <input v-model="repairOptions.reset_graphics" type="checkbox" />
          <span>
            <strong>重置画质</strong>
            <em>把渲染距离、阴影、粒子调低，会修改 options.txt</em>
          </span>
        </label>
        <label class="repair-option">
          <input v-model="repairOptions.repair_assets" type="checkbox" />
          <span>
            <strong>校验并补全资源文件</strong>
            <em>补齐缺失的声音与语言文件，可能需要下载数百 MB</em>
          </span>
        </label>
        <label class="repair-option">
          <input v-model="repairOptions.reinstall_loader" type="checkbox" />
          <span>
            <strong>加载器损坏时重装</strong>
            <em>只清加载器产物，mods 与存档保留；会重新下载依赖</em>
          </span>
        </label>

        <div class="repair-plan">
          <p class="repair-plan__title">将改动的内容</p>
          <p v-if="repairPreviewing" class="muted small">正在扫描现有文件…</p>
          <template v-else-if="repairPreview">
            <p v-if="!repairPreview.actions.length" class="muted small">
              没有需要改动的内容。
            </p>
            <div
              v-for="action in repairPreview.actions"
              :key="action.id"
              class="repair-plan__item"
            >
              <div class="repair-plan__head">
                <strong>{{ action.title }}</strong>
                <span class="muted small">
                  {{ action.count ? `${action.count} 个文件` : "自动选择" }}
                </span>
              </div>
              <p class="muted small">{{ action.detail }}</p>
              <ul v-if="action.files.length" class="repair-plan__files">
                <li v-for="file in action.files" :key="file">{{ file }}</li>
              </ul>
              <p
                v-if="action.count > action.files.length"
                class="muted small"
              >
                仅列出前 {{ action.files.length }} 项，其余 {{ action.count - action.files.length }} 项会在执行时一并处理。
              </p>
            </div>
            <p
              v-for="issue in repairPreview.issues"
              :key="issue"
              class="muted small"
            >
              {{ issue }}
            </p>
          </template>
        </div>
      </div>
      <template #footer>
        <WoodButton variant="ghost" size="sm" @click="openRepairHistory">
          修复历史
        </WoodButton>
        <WoodButton variant="ghost" size="sm" @click="repairOptionsOpen = false">取消</WoodButton>
        <WoodButton
          variant="primary"
          size="sm"
          :disabled="repairing"
          @click="repairOptionsOpen = false; runRepair()"
        >
          开始修复
        </WoodButton>
      </template>
    </WoodModal>

    <WoodModal
      v-if="repairOpen && repairResult"
      title="一键修复结果"
      :width="470"
      @close="repairOpen = false"
    >
      <div class="check-body">
        <p v-if="repairLines.length === 0" class="check-ok">
          <AppIcon name="check" :size="14" />
          没有发现需要修复的问题
        </p>
        <ul v-else class="check-list">
          <li v-for="line in repairLines" :key="line" class="check-item">
            <AppIcon name="check" :size="13" />
            <span>{{ line }}</span>
          </li>
        </ul>
        <template v-if="repairProblems.length">
          <p class="repair-err-title">
            <AppIcon name="alert" :size="13" />
            仍需手动处理
          </p>
          <ul class="check-list">
            <li v-for="line in repairProblems" :key="line" class="check-item">
              <span class="check-name">{{ line }}</span>
            </li>
          </ul>
        </template>
      </div>
      <template #footer>
        <WoodButton
          v-if="canReinstallLoader"
          variant="primary"
          size="sm"
          :disabled="reinstalling"
          @click="reinstallCurrentLoader"
        >
          {{ reinstalling ? "正在重装…" : "重装加载器" }}
        </WoodButton>
        <WoodButton
          v-if="repairResult?.backup_dir"
          variant="ghost"
          size="sm"
          :disabled="restoring"
          @click="undoRepair"
        >
          {{ restoring ? "还原中…" : "还原修复前状态" }}
        </WoodButton>
        <WoodButton variant="ghost" size="sm" @click="openRepairHistory">
          修复历史
        </WoodButton>
        <WoodButton variant="ghost" size="sm" @click="repairOpen = false">关闭</WoodButton>
        <WoodButton variant="soft" size="sm" @click="openVersionFolder">
          <AppIcon name="folder" :size="13" />
          打开版本目录
        </WoodButton>
      </template>
    </WoodModal>

    <WoodModal
      v-if="loaderUpdateOpen"
      title="更新加载器"
      :width="420"
      @close="loaderUpdateOpen = false"
    >
      <div class="check-body">
        <p class="muted small">
          当前 {{ loaderLabel }} {{ info?.loader_version || "未知" }} · 游戏版本
          {{ info?.game_version }}
        </p>
        <p v-if="loaderUpdateLoading" class="muted small">正在获取可用版本…</p>
        <div v-else-if="loaderUpdateError" class="state state--error state--inline">
          <span class="state__icon"><AppIcon name="alert" :size="16" /></span>
          <div class="state__text">
            <p class="state__title">版本列表读取失败</p>
            <p class="state__hint">{{ loaderUpdateError }}</p>
            <div class="state__actions">
              <WoodButton variant="soft" size="sm" @click="loaderUpdateVersions = []; openLoaderUpdate()">
                重试
              </WoodButton>
            </div>
          </div>
        </div>
        <p v-else-if="!loaderUpdateVersions.length" class="muted small">
          没有找到适配当前游戏版本的 {{ loaderLabel }}。
        </p>
        <div v-else class="loader-update">
          <button
            v-for="version in loaderUpdateVersions"
            :key="version.version"
            class="loader-update__item"
            :class="{ 'loader-update__item--active': loaderUpdateTarget === version.version }"
            @click="loaderUpdateTarget = version.version"
          >
            <span class="mono">{{ version.version }}</span>
            <span v-if="version.version === info?.loader_version" class="tag">当前</span>
            <span v-else-if="version.recommended" class="tag tag--grass">推荐</span>
          </button>
        </div>
        <p class="muted small">
          更新只替换加载器清单与相关库，mods、存档和配置都保留；新加载器可能与旧模组不兼容。
        </p>
      </div>
      <template #footer>
        <WoodButton variant="ghost" size="sm" @click="loaderUpdateOpen = false">取消</WoodButton>
        <WoodButton
          variant="primary"
          size="sm"
          :disabled="
            updatingLoader ||
            !loaderUpdateTarget ||
            loaderUpdateTarget === info?.loader_version
          "
          @click="applyLoaderUpdate"
        >
          {{ updatingLoader ? "正在更新…" : "更新到此版本" }}
        </WoodButton>
      </template>
    </WoodModal>

    <WoodModal
      v-if="historyOpen"
      title="修复历史"
      :width="540"
      @close="historyOpen = false"
    >
      <div class="check-body">
        <p v-if="historyLoading" class="muted small">正在读取备份…</p>
        <p v-else-if="!history.length" class="check-ok">
          还没有修复备份，执行一次一键修复后会自动记录。
        </p>
        <ul v-else class="check-list">
          <li v-for="entry in history" :key="entry.id" class="history-item">
            <div class="history-item__head">
              <strong>{{ formatBackupTime(entry.created) }}</strong>
              <span class="muted small">{{ entry.files }} 个文件</span>
            </div>
            <p v-if="entry.samples.length" class="muted small history-item__files">
              {{ entry.samples.join("、") }}{{ entry.files > entry.samples.length ? "…" : "" }}
            </p>
            <WoodButton
              variant="ghost"
              size="sm"
              :disabled="!!restoringBackupId"
              @click="restoreBackupAt(entry)"
            >
              {{ restoringBackupId === entry.id ? "还原中…" : "还原到这次修复前" }}
            </WoodButton>
          </li>
        </ul>
      </div>
      <template #footer>
        <WoodButton variant="ghost" size="sm" @click="historyOpen = false">关闭</WoodButton>
      </template>
    </WoodModal>

    <WoodModal
      v-if="packUpdateOpen && packUpdatePreview"
      title="更新整合包"
      :width="560"
      @close="packUpdateOpen = false"
    >
      <div class="check-body">
        <p class="muted small">
          {{ packUpdatePreview.name }}
          <template v-if="packUpdatePreview.old_version">
            · {{ packUpdatePreview.old_version }}
          </template>
          → {{ packUpdatePreview.new_version }}
        </p>
        <p class="muted small">
          {{ packUpdatePreview.game_version }} · {{ packUpdatePreview.loader }}
          <template v-if="packUpdatePreview.loader_version">
            {{ packUpdatePreview.loader_version }}
          </template>
        </p>
        <ul class="check-list">
          <li class="check-item">
            <span>
              新增 {{ packUpdatePreview.added }} · 更新 {{ packUpdatePreview.changed }} ·
              移除 {{ packUpdatePreview.removed }} · 覆盖 {{ packUpdatePreview.overrides }} 个配置
            </span>
          </li>
          <li class="check-item">
            <span>需要下载 {{ formatBytes(packUpdatePreview.download_bytes) }}</span>
          </li>
        </ul>

        <p
          v-for="warning in packUpdatePreview.warnings"
          :key="warning"
          class="repair-err-title"
        >
          <AppIcon name="alert" :size="13" />
          {{ warning }}
        </p>

        <div v-if="packUpdatePreview.files.length" class="pack-update-list">
          <p class="repair-plan__title">文件变动（前 {{ packUpdatePreview.files.length }} 项）</p>
          <ul class="check-list">
            <li
              v-for="file in packUpdatePreview.files"
              :key="`${file.kind}:${file.path}`"
              class="pack-update-item"
            >
              <span class="tag">{{ packUpdateKindLabel[file.kind] ?? file.kind }}</span>
              <span class="mono">{{ file.path }}</span>
            </li>
          </ul>
        </div>
        <p class="muted small">
          只会改动整合包清单里的文件，你自己添加的 Mod、存档与截图都会保留；被覆盖或删除的文件会先备份。
        </p>
      </div>
      <template #footer>
        <WoodButton variant="ghost" size="sm" @click="packUpdateOpen = false">
          {{ packUpdating ? "后台运行" : "取消" }}
        </WoodButton>
        <WoodButton
          variant="primary"
          size="sm"
          :disabled="packUpdating"
          @click="applyPackUpdate"
        >
          {{ packUpdating ? "更新中…" : "开始更新" }}
        </WoodButton>
      </template>
    </WoodModal>

    <Teleport to="body">
      <Transition name="pack-modal">
        <div v-if="showExport && exportPreview" class="pack-modal">
          <div class="pack-modal__mask" @click="showExport = false" />
          <div class="pack-modal__card">
            <header class="pack-modal__head">
              <div>
                <h2 class="pack-modal__title">导出整合包</h2>
                <p class="muted small">
                  {{ exportPreview.game_version }} · {{ exportPreview.loader }}
                  <template v-if="exportPreview.loader_version">
                    {{ exportPreview.loader_version }}
                  </template>
                </p>
              </div>
              <IconButton icon="close" label="关闭" :size="32" @click="showExport = false" />
            </header>

            <div class="pack-modal__body">
              <div class="export-two">
                <label class="pack-field">
                  <span class="pack-field__label">整合包名称</span>
                  <WoodInput v-model="exportForm.name" />
                </label>
                <label class="pack-field">
                  <span class="pack-field__label">整合包版本</span>
                  <WoodInput v-model="exportForm.version" mono />
                </label>
              </div>

              <label class="pack-field">
                <span class="pack-field__label">导出格式</span>
                <WoodSelect v-model="exportForm.format" :options="exportFormats" />
                <span class="pack-field__hint muted">
                  通用 ZIP 直接把所选内容打包；.mrpack 会按 SHA-1 匹配 Modrinth 项目写入 files[]，
                  匹配不到的文件仍打包进 overrides。
                </span>
              </label>

              <div class="export-head">
                <span class="pack-field__label">导出内容</span>
                <div class="row">
                  <button class="export-link" @click="selectAllGroups">全选</button>
                  <button class="export-link" @click="clearGroups">清空</button>
                  <span class="muted small">
                    共 {{ formatBytes(exportPreview.total_bytes) }}
                  </span>
                </div>
              </div>

              <div class="export-groups">
                <button
                  v-for="entry in exportPreview.entries"
                  :key="entry.key"
                  class="export-group"
                  :class="{ 'export-group--on': exportForm.groups.includes(entry.key) }"
                  @click="toggleExportGroup(entry.key)"
                >
                  <span class="export-group__box">
                    <AppIcon
                      v-if="exportForm.groups.includes(entry.key)"
                      name="check"
                      :size="12"
                    />
                  </span>
                  <span class="export-group__label">{{ entry.label }}</span>
                  <span class="export-group__meta mono">
                    {{ entry.file_count }} 个 · {{ formatBytes(entry.bytes) }}
                  </span>
                </button>
              </div>

              <div
                v-if="exportPreview.suggested_extras.length || customExtras.length"
                class="export-head"
              >
                <span class="pack-field__label">额外包含文件夹</span>
                <span class="muted small">TACZ 枪包等自定义内容默认不进整合包，勾选后一并打包</span>
              </div>

              <div
                v-if="exportPreview.suggested_extras.length || customExtras.length"
                class="export-groups export-groups--extras"
              >
                <button
                  v-for="entry in exportPreview.suggested_extras"
                  :key="entry.key"
                  class="export-group"
                  :class="{ 'export-group--on': exportForm.extras.includes(entry.key) }"
                  @click="toggleExtra(entry.key)"
                >
                  <span class="export-group__box">
                    <AppIcon
                      v-if="exportForm.extras.includes(entry.key)"
                      name="check"
                      :size="12"
                    />
                  </span>
                  <span class="export-group__label mono">{{ entry.key }}</span>
                  <span class="export-group__meta mono">
                    {{ entry.file_count }} 个文件 · {{ formatBytes(entry.bytes) }}
                  </span>
                </button>
                <div
                  v-for="dir in customExtras"
                  :key="dir"
                  class="export-group export-group--on export-group--custom"
                >
                  <span class="export-group__box">
                    <AppIcon name="check" :size="12" />
                  </span>
                  <span class="export-group__label mono">{{ dir }}</span>
                  <button
                    class="export-link"
                    title="移除该目录"
                    @click.stop="customExtras = customExtras.filter((item) => item !== dir)"
                  >
                    <AppIcon name="close" :size="12" />
                  </button>
                </div>
              </div>

              <div class="export-extra-add">
                <WoodInput
                  v-model="customExtraInput"
                  placeholder="添加其他文件夹，例如 config/tacz/custom"
                  @keyup.enter="addCustomExtra"
                />
                <WoodButton variant="soft" size="sm" @click="addCustomExtra">
                  <AppIcon name="plus" :size="14" />
                  添加
                </WoodButton>
              </div>

              <div class="export-target">
                <div>
                  <span class="pack-field__label">保存位置</span>
                  <p class="mono small export-target__path">
                    {{ exportForm.target || "尚未选择" }}
                  </p>
                </div>
                <WoodButton variant="soft" size="sm" @click="pickExportTarget">
                  <AppIcon name="folder" :size="14" />
                  选择位置
                </WoodButton>
              </div>
            </div>

            <footer class="pack-modal__foot">
              <span class="muted small">
                存档、截图与个人信息默认不导出，可在上方手动勾选。
              </span>
              <div class="row">
                <WoodButton variant="ghost" @click="showExport = false">取消</WoodButton>
                <WoodButton
                  variant="primary"
                  sheen
                  :disabled="!!store.installingId || !exportForm.groups.length || !exportForm.target"
                  @click="startExport"
                >
                  <AppIcon name="download" :size="15" />
                  开始导出
                </WoodButton>
              </div>
            </footer>
          </div>
        </div>
      </Transition>
    </Teleport>
  </div>
</template>

<style scoped>
.ins-head {
  align-items: center;
  justify-content: space-between;
}

.ins-tile {
  width: 46px;
  height: 46px;
  border-radius: 2px;
  display: grid;
  place-items: center;
  background: linear-gradient(
    160deg,
    color-mix(in srgb, var(--tile) 14%, #fffdf6),
    color-mix(in srgb, var(--tile) 28%, #efe6d4)
  );
  border: 1px solid color-mix(in srgb, var(--tile) 32%, transparent);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.75), var(--shadow-1);
  flex-shrink: 0;
  overflow: hidden;
}

.ins-tile__img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.ins-head__text {
  flex: 1;
  min-width: 0;
}

.ins-head__actions {
  flex-shrink: 0;
}

.ins-loading {
  display: flex;
  align-items: center;
  gap: var(--space-4);
}

.ins-loading .skeleton {
  width: 140px;
  height: 12px;
}

.ins-layout {
  display: grid;
  grid-template-columns: 190px minmax(0, 1fr);
  gap: var(--space-6);
  align-items: start;
}

/* 侧栏和联机页一致：行距收紧、纯色块高亮，不做按键浮雕 */
.ins-side {
  padding: var(--space-4);
  display: flex;
  flex-direction: column;
  gap: 2px;
  position: sticky;
  top: 0;
  /* 整个侧栏当一只托盘：2px 外框 + 内衬一圈受光细边 */
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--panel) 46%, #fff);
}

.ins-side__item {
  display: flex;
  align-items: center;
  gap: 9px;
  width: 100%;
  height: 34px;
  padding: 0 11px;
  border-radius: 0;
  color: var(--ink-soft);
  font-size: var(--text-lg);
  font-weight: 600;
  text-align: left;
  transition: background var(--dur-1) ease, color var(--dur-1) ease,
    box-shadow var(--dur-1) ease;
}

.ins-side__item .icon {
  color: var(--muted);
  transition: color var(--dur-1) ease;
}

.ins-side__item:hover {
  background: rgba(var(--accent-rgb), 0.07);
  color: var(--walnut-deep);
}

.ins-side__item:hover .icon {
  color: var(--walnut);
}

.ins-side__item--active,
.ins-side__item--active:hover {
  background: var(--moss-soft);
  color: var(--moss);
  /* 选中项左侧压一条 3px 实心竖条，像像素画里的选中标记 */
  box-shadow: inset 3px 0 0 var(--moss);
}

.ins-side__item--active .icon,
.ins-side__item--active:hover .icon {
  color: var(--moss);
}

.ins-side__label {
  flex: 1;
}

.ins-side__count {
  font-size: var(--text-xs);
  color: var(--muted);
  letter-spacing: 0.08em;
}

.ins-side__meta {
  margin-top: 14px;
  padding: 10px 11px;
  border-top: 1px dashed var(--line);
  display: flex;
  flex-direction: column;
  gap: 5px;
}

.ins-side__key {
  font-family: var(--font-mono);
  font-size: var(--text-xs);
  letter-spacing: 0.14em;
  color: var(--muted);
  text-transform: uppercase;
}

.ins-side__path {
  font-size: var(--text-xs);
  color: var(--ink-soft);
  word-break: break-all;
  line-height: 1.5;
}

.ins-side__fav {
  align-self: flex-start;
}

.ins-main {
  display: flex;
  flex-direction: column;
  gap: var(--space-6);
  min-width: 0;
}

.group {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.group-head {
  display: flex;
  align-items: center;
  gap: var(--space-5);
  margin-bottom: 10px;
}

.group-titles {
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.group-line {
  flex: 1;
  height: 1px;
  background: linear-gradient(90deg, var(--line), transparent);
}

.setting {
  display: flex;
  align-items: center;
  gap: var(--space-row-gap);
  padding: var(--space-row) 0;
  border-bottom: 1px dashed var(--line);
}

.setting--last {
  border-bottom: none;
}

/* 长参数输入改为上下布局，避免和标签互相挤压 */
.setting--stack {
  flex-direction: column;
  align-items: stretch;
  gap: var(--space-3);
}

.setting--stack .setting__label {
  width: auto;
}

.setting--stack .setting__control {
  justify-content: stretch;
}

.setting__label {
  width: 200px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.setting__label strong {
  font-size: var(--text-lg);
  color: var(--walnut-deep);
}

.setting__label span {
  font-size: var(--text-sm);
  color: var(--muted);
  line-height: 1.45;
}

.setting__control {
  flex: 1;
  min-width: 0;
  display: flex;
  justify-content: flex-end;
}

.setting__control--wide {
  justify-content: stretch;
}

.setting__control--wide > * {
  width: 100%;
}

.resolution {
  width: 100%;
}

.resolution__x {
  flex-shrink: 0;
}

.ins-textarea {
  width: 100%;
  padding: 10px 13px;
  border-radius: 0;
  border-style: solid;
  border-width: 1px;
  /* 凹槽：上左压暗、下右受光，配内阴影 */
  border-top-color: color-mix(in srgb, var(--line-strong) 64%, #000);
  border-left-color: color-mix(in srgb, var(--line-strong) 64%, #000);
  border-right-color: color-mix(in srgb, var(--line-strong) 38%, #fff);
  border-bottom-color: color-mix(in srgb, var(--line-strong) 38%, #fff);
  background: var(--panel);
  box-shadow: inset 0 1px 2px rgba(60, 40, 20, 0.1);
  color: var(--ink);
  font-size: var(--text-lg);
  line-height: 1.5;
  resize: vertical;
  transition: border-color var(--dur-2) ease, box-shadow var(--dur-2) ease;
}

.ins-textarea:focus {
  outline: none;
  border-color: var(--moss);
  box-shadow: 0 0 0 3px rgba(94, 123, 73, 0.14);
}

.icon-grid {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-3);
}

.icon-chip {
  display: flex;
  align-items: center;
  gap: 7px;
  height: 36px;
  padding: 0 10px 0 6px;
  border-radius: 0;
  border-style: solid;
  border-width: 1px 1px 2px;
  border-top-color: color-mix(in srgb, var(--frame) 62%, #fff);
  border-left-color: color-mix(in srgb, var(--frame) 62%, #fff);
  border-right-color: color-mix(in srgb, var(--frame) 72%, #000);
  border-bottom-color: color-mix(in srgb, var(--frame) 72%, #000);
  background: var(--panel);
  color: var(--ink-soft);
  font-size: var(--text-base);
  transition: border-color var(--dur-2) ease, color var(--dur-2) ease,
    transform var(--dur-2) var(--ease-out-expo);
}

.icon-chip:hover {
  transform: translateY(-1px);
  border-top-color: color-mix(in srgb, var(--frame) 74%, #fff);
  border-left-color: color-mix(in srgb, var(--frame) 74%, #fff);
  border-right-color: color-mix(in srgb, var(--frame) 82%, #000);
  border-bottom-color: color-mix(in srgb, var(--frame) 82%, #000);
}

.icon-chip--active,
.icon-chip--active:hover {
  background: var(--moss-soft);
  border-top-color: color-mix(in srgb, var(--moss) 52%, #000);
  border-left-color: color-mix(in srgb, var(--moss) 52%, #000);
  border-right-color: color-mix(in srgb, var(--moss) 34%, #fff);
  border-bottom-color: color-mix(in srgb, var(--moss) 34%, #fff);
  color: var(--walnut-deep);
  transform: translateY(1px);
  box-shadow: var(--shadow-inset-soft);
}

.icon-chip__tile {
  width: 24px;
  height: 24px;
  border-radius: 0;
  display: grid;
  place-items: center;
  background: linear-gradient(
    160deg,
    color-mix(in srgb, var(--tile) 12%, #fffdf6),
    color-mix(in srgb, var(--tile) 24%, #efe6d4)
  );
  border: 1px solid color-mix(in srgb, var(--tile) 30%, transparent);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.7);
}

.icon-chip__img {
  width: 100%;
  height: 100%;
  border-radius: 0;
  object-fit: cover;
}

.quick-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
  gap: var(--space-3);
}

.detail-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(190px, 1fr));
  gap: 10px 16px;
  padding-bottom: 12px;
  border-bottom: 1px dashed var(--line);
}

.detail {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.detail__label {
  font-family: var(--font-mono);
  font-size: 9.5px;
  letter-spacing: 0.12em;
  color: var(--muted);
  text-transform: uppercase;
}

.detail__value {
  font-size: var(--text-lg);
  color: var(--ink);
}

.detail__value--row {
  display: flex;
  align-items: baseline;
  gap: 8px;
  flex-wrap: wrap;
}

.detail__action {
  font-size: var(--text-sm);
  font-family: inherit;
  color: var(--moss);
  background: none;
  border: 0;
  padding: 0;
  cursor: pointer;
  text-decoration: underline;
  text-underline-offset: 2px;
}

.detail__action:hover {
  color: var(--ink);
}

.detail__action:disabled {
  color: var(--muted);
  cursor: default;
}

.loader-update {
  display: flex;
  flex-direction: column;
  max-height: 260px;
  overflow-y: auto;
  border: 1px solid var(--line);
  background: var(--panel);
}

.loader-update__item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 10px;
  background: none;
  border: 0;
  border-bottom: 1px dashed var(--line);
  font-size: var(--text-base);
  color: var(--ink);
  text-align: left;
  cursor: pointer;
}

.loader-update__item:last-child {
  border-bottom: 0;
}

.loader-update__item:hover {
  background: var(--panel-2);
}

.loader-update__item--active {
  background: var(--moss-soft);
}

.count-strip {
  display: grid;
  grid-template-columns: repeat(5, minmax(0, 1fr));
  gap: var(--space-2);
  margin-top: 12px;
}

.count {
  display: flex;
  flex-direction: column;
  gap: 1px;
  padding: 8px 10px;
  border-radius: 0;
  border: 1px solid var(--line);
  background: var(--panel);
  /* 指标块做成浅凹槽，和输入框同一套语义 */
  box-shadow: inset 0 1px 2px rgba(60, 40, 20, 0.07);
  text-align: left;
  transition: background var(--dur-2) ease, border-color var(--dur-2) ease;
}

.count:hover {
  border-color: var(--line-strong);
  background: var(--hover);
}

.count__num {
  font-size: 24px;
  line-height: 1.05;
  color: var(--walnut-deep);
}

.count__label {
  font-size: var(--text-xs);
  color: var(--muted);
}

.file-state {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 14px 2px;
}

.file-search {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  margin: 4px 0;
}

.world-pick {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  margin: 10px 0 4px;
  padding: 10px 12px;
  border: 1px dashed var(--line);
  border-radius: 4px;
  background: rgba(var(--accent-rgb), 0.03);
}

.proj-tip {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  margin: 10px 0 4px;
  padding: 10px 12px;
  border: 1px solid var(--line);
  border-radius: 4px;
  background: rgba(var(--accent-rgb), 0.05);
}

.proj-tip__icon {
  display: grid;
  place-items: center;
  width: 32px;
  height: 32px;
  border-radius: 4px;
  flex-shrink: 0;
  background: rgba(var(--accent-rgb), 0.12);
  color: var(--moss);
}

.proj-tip__main {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
  flex: 1;
}

.proj-tip__main strong {
  font-size: var(--text-base);
  color: var(--ink);
}

.proj-tip__status {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
  font-size: var(--text-sm);
  color: var(--muted);
}

.proj-tip__status--on {
  color: var(--moss);
}

.proj-tip .row {
  flex-shrink: 0;
}

.seed-loading {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  padding: 10px 0;
}

.seed-body {
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
}

.seed-world {
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.seed-world strong {
  font-size: var(--text-lg);
  color: var(--ink);
}

.seed-value {
  padding: 12px 14px;
  border-radius: 4px;
  border: 1px dashed var(--line-strong);
  background: var(--panel-2);
}

.seed-value__text {
  font-size: 19px;
  letter-spacing: 0.04em;
  color: var(--walnut-deep);
  word-break: break-all;
}

.seed-hint {
  margin: 0;
  line-height: 1.6;
}

.check-loading {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  padding: 10px 0;
}

.check-body {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.check-ok {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  margin: 0;
  font-size: var(--text-lg);
  color: var(--ink);
}

.repair-err-title {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  margin: 4px 0 0;
  font-size: var(--text-base);
  color: #b4483c;
}

.check-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  margin: 0;
  padding: 0;
  list-style: none;
  max-height: 260px;
  overflow-y: auto;
}

.repair-option {
  display: flex;
  align-items: flex-start;
  gap: 9px;
  padding: 8px 10px;
  border: 1px solid var(--line);
  border-radius: 4px;
  background: var(--panel-2);
  cursor: pointer;
}

.repair-option input {
  margin-top: 3px;
  accent-color: var(--moss);
}

.repair-option span {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.repair-option strong {
  font-size: var(--text-base);
  color: var(--walnut-deep);
}

.repair-option em {
  font-style: normal;
  font-size: var(--text-sm);
  line-height: 1.6;
  color: var(--muted);
}

.repair-plan {
  margin-top: 12px;
  padding-top: 10px;
  border-top: 1px solid var(--line);
  max-height: 320px;
  overflow-y: auto;
}

.repair-plan__title {
  margin: 0 0 6px;
  font-size: var(--text-base);
  color: var(--walnut-deep);
}

.repair-plan__item {
  margin-top: 10px;
}

.repair-plan__item:first-of-type {
  margin-top: 0;
}

.repair-plan__head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--space-3);
}

.repair-plan__head strong {
  font-size: var(--text-base);
  color: var(--ink);
}

.repair-plan__item p {
  margin: 2px 0 0;
  line-height: 1.6;
}

.repair-plan__files {
  margin: 4px 0 0;
  padding-left: 18px;
  font-size: var(--text-sm);
  line-height: 1.7;
  color: var(--muted);
  word-break: break-all;
}

.history-item {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  padding: 9px 11px;
  border: 1px solid var(--line);
  background: var(--panel-2);
}

.history-item__head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--space-3);
}

.history-item__head strong {
  font-size: var(--text-base);
  color: var(--walnut-deep);
}

.history-item__files {
  margin: 0;
  line-height: 1.6;
  word-break: break-all;
}

.pack-update-list {
  margin-top: 10px;
  padding-top: 8px;
  border-top: 1px solid var(--line);
}

.pack-update-item {
  display: flex;
  align-items: baseline;
  gap: var(--space-3);
  word-break: break-all;
}

.check-item {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: 7px 10px;
  border-radius: 4px;
  border: 1px solid var(--line);
  background: var(--panel-2);
  font-size: var(--text-base);
}

.check-tag {
  flex-shrink: 0;
  padding: 1px 6px;
  border-radius: 3px;
  border: 1px solid var(--line);
  color: var(--ink-soft);
  font-size: var(--text-sm);
}

.check-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--ink);
}

.world-pick__label {
  font-size: var(--text-base);
  font-weight: 700;
  color: var(--ink-soft);
  flex-shrink: 0;
}

.world-pick :deep(.select) {
  min-width: 220px;
}

.file-search :deep(.input) {
  max-width: 320px;
}

.file-state .skeleton {
  width: 100px;
  height: 12px;
}

.file-empty {
  display: flex;
  align-items: center;
  gap: var(--space-6);
  padding: 24px 10px;
  color: var(--walnut);
}

.file-empty__title {
  font-size: var(--text-lg);
  font-weight: 800;
  color: var(--walnut-deep);
  margin-bottom: 2px;
}

.file-empty > div {
  flex: 1;
}

.file-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
  margin-top: 4px;
}

/* 长列表分页的「显示更多」：截图网格里让它占满一行 */
.file-more {
  display: flex;
  justify-content: center;
  padding: 12px 0 4px;
  grid-column: 1 / -1;
}

.file-row {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  padding: 8px 9px;
  border-radius: 0;
  /* 屏幕外的行跳过布局与绘制，长列表滚动时内存和帧率都更稳 */
  content-visibility: auto;
  contain-intrinsic-size: auto 47px;
  transition: background var(--dur-1) ease, box-shadow var(--dur-1) ease;
}

.file-row:hover {
  background: var(--hover);
  box-shadow: inset 3px 0 0 var(--moss);
}

.file-row__icon {
  width: 30px;
  height: 30px;
  border-radius: 3px;
  display: grid;
  place-items: center;
  background: var(--moss-soft);
  color: var(--moss);
  flex-shrink: 0;
}

.file-row__thumb {
  width: 34px;
  height: 34px;
  border-radius: 3px;
  overflow: hidden;
  border: 1px solid var(--line);
  background: var(--panel-2);
  flex-shrink: 0;
}

.file-row__thumb img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  image-rendering: pixelated;
}

/* Mod 图标来自项目页，保持原色不做像素化放大 */
.file-row__thumb--mod {
  display: grid;
  place-items: center;
  color: var(--walnut);
  background: var(--panel);
}

.file-row__thumb--mod img {
  object-fit: contain;
  image-rendering: auto;
}

.shot-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
  gap: var(--space-5);
  padding-top: 4px;
}

.shot-card {
  display: flex;
  flex-direction: column;
  border-style: solid;
  border-width: 2px;
  border-top-color: color-mix(in srgb, var(--frame) 66%, #fff);
  border-left-color: color-mix(in srgb, var(--frame) 66%, #fff);
  border-right-color: color-mix(in srgb, var(--frame) 74%, #000);
  border-bottom-color: color-mix(in srgb, var(--frame) 74%, #000);
  border-radius: 0;
  overflow: hidden;
  background: var(--panel-2);
  /* 截图同样按需绘制，避免一屏之外的缩略图常驻合成层 */
  content-visibility: auto;
  contain-intrinsic-size: auto 190px;
  transition: border-color var(--dur-2) ease, transform var(--dur-2) var(--ease-out-expo),
    box-shadow var(--dur-2) ease;
}

.shot-card:hover {
  transform: translateY(-2px);
  border-top-color: color-mix(in srgb, var(--frame) 78%, #fff);
  border-left-color: color-mix(in srgb, var(--frame) 78%, #fff);
  border-right-color: color-mix(in srgb, var(--frame) 86%, #000);
  border-bottom-color: color-mix(in srgb, var(--frame) 86%, #000);
  box-shadow: var(--shadow-1);
}

.shot-card__thumb {
  display: block;
  width: 100%;
  aspect-ratio: 16 / 10;
  padding: 0;
  border: 0;
  background: var(--panel-2);
  cursor: zoom-in;
  overflow: hidden;
}

.shot-card__thumb img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  transition: transform var(--dur-3) var(--ease-out-expo);
}

.shot-card__thumb:hover img {
  transform: scale(1.04);
}

.shot-card__blank {
  width: 100%;
  height: 100%;
  display: grid;
  place-items: center;
  color: var(--muted);
}

.shot-card__foot {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: 6px 8px 6px 10px;
}

.shot-card__name {
  flex: 1;
  min-width: 0;
  font-size: var(--text-sm);
  color: var(--ink-soft);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.file-row__main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.file-row__name {
  font-size: var(--text-lg);
  color: var(--ink);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.file-row__off {
  margin-left: 6px;
  font-size: 9.5px;
  color: var(--muted);
  border-color: var(--line);
}

.file-row__meta {
  font-size: var(--text-xs);
  color: var(--muted);
}

@media (max-width: 1080px) {
  .ins-layout {
    grid-template-columns: 158px minmax(0, 1fr);
  }

  .setting__label {
    width: 160px;
  }

  .count-strip {
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }
}

/* 导出弹层 */
.pack-modal {
  position: fixed;
  inset: 0;
  z-index: 140;
  display: grid;
  place-items: center;
}

.pack-modal__mask {
  position: absolute;
  inset: 0;
  background: rgba(28, 20, 12, 0.38);
  backdrop-filter: blur(3px);
}

.pack-modal__card {
  position: relative;
  width: min(620px, calc(100vw - 56px));
  max-height: calc(100vh - 72px);
  overflow-y: auto;
  padding: 20px 22px;
  border-radius: 0;
  border-style: solid;
  border-width: 2px;
  border-top-color: color-mix(in srgb, var(--frame) 68%, #fff);
  border-left-color: color-mix(in srgb, var(--frame) 68%, #fff);
  border-right-color: color-mix(in srgb, var(--frame) 76%, #000);
  border-bottom-color: color-mix(in srgb, var(--frame) 76%, #000);
  background: rgba(var(--glass-rgb), 0.97);
  box-shadow: var(--shadow-2);
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.pack-modal__head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--space-5);
}

.pack-modal__title {
  font-family: var(--font-display);
  font-size: 28px;
  line-height: 1.1;
  color: var(--walnut-deep);
  margin: 2px 0;
}

.pack-modal__body {
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
}

.pack-field {
  display: flex;
  flex-direction: column;
  gap: 5px;
}

.pack-field__label {
  font-size: var(--text-sm);
  color: var(--muted);
  letter-spacing: 0.04em;
}

.export-two {
  display: grid;
  grid-template-columns: minmax(0, 1.4fr) minmax(0, 1fr);
  gap: var(--space-5);
}

.export-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-5);
  margin-top: 2px;
}

.export-link {
  font-size: var(--text-sm);
  color: var(--moss);
  padding: 2px 4px;
  border-radius: 4px;
}

.export-link:hover {
  background: rgba(var(--accent-rgb), 0.08);
}

.export-groups {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 5px;
}

/* 额外包含文件夹区：整体加一条淡边框与常规组区分 */
.export-groups--extras {
  padding: var(--space-3);
  border: 1px dashed var(--line-strong);
  border-radius: 4px;
}

.export-group--custom {
  cursor: default;
}

.export-extra-add {
  display: flex;
  gap: var(--space-2);
  margin-top: -6px;
}

.pack-field__hint {
  margin-top: 4px;
  font-size: var(--text-sm);
  line-height: 1.5;
}

.export-group {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: 8px 9px;
  border-style: solid;
  border-width: 1px 1px 2px;
  border-top-color: color-mix(in srgb, var(--frame) 60%, #fff);
  border-left-color: color-mix(in srgb, var(--frame) 60%, #fff);
  border-right-color: color-mix(in srgb, var(--frame) 70%, #000);
  border-bottom-color: color-mix(in srgb, var(--frame) 70%, #000);
  border-radius: 0;
  background: var(--panel);
  text-align: left;
  color: var(--ink-soft);
  transition: border-color var(--dur-2) ease, transform var(--dur-2) var(--ease-out-expo),
    box-shadow var(--dur-2) ease;
}

.export-group:hover {
  transform: translateY(-1px);
  border-top-color: color-mix(in srgb, var(--frame) 72%, #fff);
  border-left-color: color-mix(in srgb, var(--frame) 72%, #fff);
  border-right-color: color-mix(in srgb, var(--frame) 80%, #000);
  border-bottom-color: color-mix(in srgb, var(--frame) 80%, #000);
}

.export-group--on {
  background: var(--moss-soft);
  border-top-color: color-mix(in srgb, var(--moss) 52%, #000);
  border-left-color: color-mix(in srgb, var(--moss) 52%, #000);
  border-right-color: color-mix(in srgb, var(--moss) 34%, #fff);
  border-bottom-color: color-mix(in srgb, var(--moss) 34%, #fff);
  color: var(--walnut-deep);
  transform: translateY(1px);
  box-shadow: var(--shadow-inset-soft);
}

.export-group__box {
  width: 15px;
  height: 15px;
  border-radius: 0;
  border: 1px solid var(--line-strong);
  display: grid;
  place-items: center;
  color: #fbf6ea;
  flex-shrink: 0;
  transition: background var(--dur-1) ease, border-color var(--dur-1) ease;
}

.export-group--on .export-group__box {
  background: var(--moss);
  border-color: var(--moss);
}

.export-group__label {
  flex: 1;
  font-size: var(--text-base);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.export-group__meta {
  font-size: var(--text-xs);
  color: var(--muted);
  white-space: nowrap;
}

.export-target {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 14px;
  padding-top: 10px;
  border-top: 1px dashed var(--line);
}

.export-target__path {
  margin-top: 3px;
  color: var(--ink-soft);
  word-break: break-all;
}

.pack-modal__foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 14px;
  padding-top: 10px;
  border-top: 1px dashed var(--line);
}

.pack-modal__foot > .muted {
  max-width: 300px;
  line-height: 1.5;
}

.pack-modal-enter-active,
.pack-modal-leave-active {
  transition: opacity var(--dur-1) ease;
}

.pack-modal-enter-active .pack-modal__card,
.pack-modal-leave-active .pack-modal__card {
  transition: transform var(--dur-2) var(--ease-out-expo);
}

.pack-modal-enter-from,
.pack-modal-leave-to {
  opacity: 0;
}

.pack-modal-enter-from .pack-modal__card,
.pack-modal-leave-to .pack-modal__card {
  transform: translateY(10px) scale(0.98);
}

/* Mod 更新检测 */
.update-state {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 10px 12px;
  border-radius: 4px;
  border: 1px dashed var(--line-strong);
  background: var(--panel-2);
  color: var(--muted);
  font-size: var(--text-base);
}

.update-state--err {
  color: var(--danger);
  background: var(--danger-soft);
}

.update-state--err .btn,
.update-state--err button:last-child {
  margin-left: auto;
}

.update-panel {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
  padding: 12px 14px;
  border-radius: 2px;
  border: 2px solid var(--frame);
  border-top-color: color-mix(in srgb, var(--frame) 66%, #fff);
  border-left-color: color-mix(in srgb, var(--frame) 66%, #fff);
  border-right-color: color-mix(in srgb, var(--frame) 74%, #000);
  border-bottom-color: color-mix(in srgb, var(--frame) 74%, #000);
  background: linear-gradient(180deg, rgba(var(--accent-rgb), 0.06), transparent);
}

.update-panel__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-5);
  flex-wrap: wrap;
}

.update-panel__title {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.update-panel__title strong {
  font-size: var(--text-lg);
  font-weight: 700;
  color: var(--walnut-deep);
}

.update-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.update-row {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  padding: 8px 10px;
  border-radius: 4px;
  border: 1px solid var(--line);
  background: var(--panel);
}

.update-row__icon {
  display: grid;
  place-items: center;
  width: 32px;
  height: 32px;
  border-radius: 4px;
  background: var(--moss-soft);
  color: var(--moss);
  flex-shrink: 0;
}

.update-row__main {
  display: flex;
  flex: 1;
  min-width: 0;
  flex-direction: column;
  gap: 1px;
}

.update-row__main strong {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: var(--text-base);
  color: var(--walnut-deep);
}

.update-row__link {
  display: inline-grid;
  place-items: center;
  color: var(--muted);
  padding: 0;
  transition: color var(--dur-1) ease;
}

.update-row__link:hover {
  color: var(--moss);
}

.update-row__file {
  font-size: var(--text-xs);
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.update-notes {
  margin: 0;
  padding-left: 18px;
  display: flex;
  flex-direction: column;
  gap: 2px;
  font-size: var(--text-sm);
  line-height: 1.6;
  color: var(--muted);
}

.opts-save {
  align-items: center;
}

.opts-save .input {
  flex: 1;
  min-width: 0;
}

.opts-presets {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.opts-presets__head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--space-5);
  flex-wrap: wrap;
}

.opts-presets__head strong {
  font-size: var(--text-lg);
  color: var(--walnut-deep);
}

.opts-preset {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-5);
  padding: 9px 11px;
  border-radius: 0;
  border: 1px solid var(--line);
  background: var(--panel-2);
  box-shadow: inset 0 1px 2px rgba(60, 40, 20, 0.06);
}

.opts-preset__info {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.opts-preset__info strong {
  font-size: var(--text-base);
  color: var(--ink);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 存档备份面板 */
.file-toggle {
  flex-shrink: 0;
  margin: 0 2px;
}

.save-backups {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
  padding: var(--space-5);
  border-radius: 2px;
  border: 1px solid var(--line);
  background: var(--panel-2);
  box-shadow: inset 0 1px 2px rgba(70, 48, 26, 0.06);
}

.save-backups__head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--space-5);
}

.save-backups__titles {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.save-backups__titles strong {
  font-size: var(--text-lg);
  color: var(--walnut-deep);
}

.save-backups__empty {
  margin: 2px 0 0;
}

.save-backups__list {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.backup-row {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  padding: 8px 10px;
  border-radius: 2px;
  border: 1px solid var(--line);
  background: var(--panel);
}

.backup-row__icon {
  display: grid;
  place-items: center;
  width: 28px;
  height: 28px;
  flex-shrink: 0;
  border-radius: 2px;
  background: var(--moss-soft);
  color: var(--moss);
}

.backup-row__main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.backup-row__main strong {
  font-size: var(--text-base);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* ---------- config 文本编辑器 ---------- */
.cfg {
  display: grid;
  grid-template-columns: 260px minmax(0, 1fr);
  gap: var(--space-5);
  margin-top: 12px;
}

.cfg__side {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  min-width: 0;
}

.cfg__list {
  flex: 1;
  min-height: 420px;
  max-height: 520px;
  /* 只纵向滚动：缩进与长文件名由 ellipsis 处理，不再撑出横向滚动条 */
  overflow-y: auto;
  overflow-x: hidden;
  border: 1px solid var(--line-strong);
  background: var(--panel-2);
  box-shadow: inset 0 1px 2px rgba(60, 40, 20, 0.1);
}

.cfg__dir {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  width: 100%;
  height: 26px;
  padding-right: 8px;
  color: var(--muted);
  font-size: var(--text-sm);
  font-weight: 600;
  text-align: left;
  overflow: hidden;
}

.cfg__dir:hover {
  background: var(--hover);
  color: var(--ink-soft);
}

.cfg__chev {
  flex-shrink: 0;
  transition: transform var(--dur-1) var(--ease-out-expo);
}

/* 收起时箭头转向右侧，和常见文件树一致 */
.cfg__chev--collapsed {
  transform: rotate(-90deg);
}

.cfg__file {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  width: 100%;
  height: 26px;
  padding-right: 8px;
  color: var(--ink-soft);
  font-size: var(--text-base);
  text-align: left;
  overflow: hidden;
}

.cfg__file:hover {
  background: var(--hover);
}

.cfg__file--active {
  background: var(--moss-soft);
  color: var(--moss);
}

.cfg__name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cfg__dot {
  width: 6px;
  height: 6px;
  flex-shrink: 0;
  border-radius: 50%;
  background: var(--amber);
}

.cfg__main {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  min-width: 0;
}

.cfg__bar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px 10px;
}

.cfg__path {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  color: var(--ink-soft);
  font-size: var(--text-sm);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cfg__meta,
.cfg__actions {
  flex-shrink: 0;
}

.cfg__editor {
  width: 100%;
  height: 480px;
  resize: vertical;
  padding: 10px 12px;
  border: 1px solid var(--line-strong);
  border-radius: 0;
  background: var(--paper-2);
  color: var(--ink-soft);
  font-family: var(--font-mono);
  font-size: var(--text-sm);
  line-height: 1.6;
  tab-size: 2;
  white-space: pre;
  overflow: auto;
  user-select: text;
}

.cfg__editor--empty {
  background: var(--panel-2);
}

.cfg__editor:focus {
  outline: none;
  border-color: var(--moss);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--moss) 20%, transparent);
}

@media (max-width: 900px) {
  .cfg {
    grid-template-columns: 1fr;
  }

  .cfg__list {
    min-height: 160px;
    max-height: 240px;
  }
}
</style>
