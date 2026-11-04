<script setup lang="ts">
import { computed, nextTick, onMounted, reactive, ref, watch } from "vue";
import { confirm, open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  api,
  openExternal,
  openFolder,
  revealFile,
  savePackDialog,
  type ExportPreview,
  type GameOptionsPreset,
  type InstanceCheck,
  type InstanceFile,
  type InstanceInfo,
  type InstanceMeta,
  type InstanceModMeta,
  type InstancePaths,
  type RepairReport,
  type ModUpdate,
  type ModUpdateReport,
  type WorldSeed,
  type WorldSummary,
} from "../api";
import AppIcon from "../components/AppIcon.vue";
import IconButton from "../components/IconButton.vue";
import InstanceGlyph from "../components/InstanceGlyph.vue";
import WoodButton from "../components/WoodButton.vue";
import WoodInput from "../components/WoodInput.vue";
import WoodModal from "../components/WoodModal.vue";
import WoodSelect from "../components/WoodSelect.vue";
import {
  exportModpack,
  launchVersion,
  notify,
  openCommunity,
  openCrashDialog,
  refreshInstanceMetas,
  reinstallLoader,
  repairInstance,
  store,
} from "../store";
import { ICON_PRESETS, formatBytes, formatDate, kindLabel } from "../utils";

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
  | "screenshots";

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
const projectionMods = ref<{ litematica: boolean; worldedit: boolean } | null>(null);
const projectionLoading = ref(false);
const seedOpen = ref(false);
const seedBusy = ref(false);
const seedInfo = ref<WorldSeed | null>(null);
const confirmName = ref<string | null>(null);
let confirmTimer: number | undefined;

const modUpdates = ref<ModUpdateReport | null>(null);
const updatesLoading = ref(false);
const updateError = ref("");
const updatingKey = ref("");

const showExport = ref(false);
const exportPreview = ref<ExportPreview | null>(null);
const exportBusy = ref(false);
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
    label: `Java ${java.major} · ${java.source}`,
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

const loaderLabel = computed(() => {
  const loader = info.value?.loader ?? "vanilla";
  if (loader === "vanilla") return "原版";
  return loader.charAt(0).toUpperCase() + loader.slice(1);
});

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
});

watch(
  () => store.contentRevision,
  () => {
    if (!store.instanceId) return;
    void loadAll();
    if (contentTabs.includes(tab.value)) void loadFiles();
  }
);

watch(tab, async (value) => {
  fileQuery.value = "";
  confirmName.value = null;
  if (value === "gameopts") void loadOptionsPresets();
  if (!contentTabs.includes(value)) return;
  if (value === "schematics") void loadProjectionMods();
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

/// 逐张按需生成缩略图，避免一次性解码整个截图目录卡住界面。
async function loadThumbs() {
  if (!store.instanceId || !["saves", "screenshots"].includes(contentKind.value)) return;
  const versionId = store.instanceId;
  const kind = contentKind.value;
  for (const file of files.value) {
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

/// 一键自愈：修清单、补缺失文件、重解本地库、禁用重复模组
async function runRepair() {
  const versionId = store.instanceId;
  if (repairing.value || !versionId) return;
  repairing.value = true;
  repairResult.value = null;
  try {
    const result = await repairInstance(versionId);
    if (!result) return;
    repairResult.value = result;
    repairOpen.value = true;
    await refreshFiles();
    if (result.disabled_mods.length) notify("已禁用重复的模组，重新启动后生效");
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
  if (result.missing_deps.length)
    lines.push(`缺少前置模组：${result.missing_deps.slice(0, 6).join("、")}`);
  return lines;
});

/// 修复结果里仍然需要手动处理的问题
const repairProblems = computed(() => {
  const result = repairResult.value;
  if (!result) return [];
  return [...result.issues, ...result.files_failed];
});

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
  if (!store.instanceId) return;
  try {
    await api.instanceToggleFile(store.instanceId, contentKind.value, file.name);
    await loadFiles();
  } catch (err) {
    notify(`切换失败：${String(err)}`, "err");
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

function contentIcon() {
  switch (tab.value) {
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
}

function downloadMore() {
  if (!info.value) return;
  const map: Record<string, string> = {
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

function launch() {
  if (info.value) launchVersion(info.value.id);
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
          <template v-if="info"> · {{ info.game_version }} · {{ kindLabel(info.kind) }} · {{ loaderLabel }}</template>
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
          @click="runRepair"
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
          @click="launch"
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
              <WoodButton variant="soft" :disabled="exportBusy" @click="openExport">
                <AppIcon name="download" :size="15" />
                导出整合包
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
                <span class="detail__value">{{ loaderLabel }}</span>
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
            留空表示跟随全局设置；这里的值只对当前版本生效，下次启动时使用。
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
              <span>留空跟随全局；自定义时不追加任何 GC 参数</span>
            </div>
            <div class="setting__control">
              <WoodSelect v-model="form.memoryMode" :options="memoryModeOptions" />
            </div>
          </div>

          <div class="setting">
            <div class="setting__label">
              <strong>Java 运行时</strong>
              <span>该版本独用的 Java 路径</span>
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
              <span>留空使用全局分辨率</span>
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
              <WoodButton variant="ghost" size="sm" @click="openContentFolder">
                <AppIcon name="folder" :size="14" />
                打开文件夹
              </WoodButton>
              <WoodButton v-if="!noWorlds" variant="soft" size="sm" @click="importContent">
                <AppIcon name="plus" :size="14" />
                从文件导入
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
                        ? "在游戏里新建世界，或从本地导入存档文件夹。"
                        : tab === "schematics"
                          ? "把 .litematic / .schem / .schematic 投影文件放进这里，游戏内用 Mod 载入。"
                          : `点击「下载新${contentLabel}」去资源站挑选，或者从本地文件导入。`
                }}
              </p>
            </div>
            <WoodButton v-if="!noWorlds" variant="soft" size="sm" @click="importContent">
              <AppIcon name="plus" :size="14" />
              从文件导入
            </WoodButton>
          </div>
          <div v-else-if="!filteredFiles.length" class="file-state">
            <span class="muted small">没有匹配的文件</span>
          </div>
          <div v-else-if="tab === 'screenshots'" class="shot-grid">
            <article v-for="file in filteredFiles" :key="file.name" class="shot-card">
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
          </div>
          <div v-else class="file-list">
            <div v-for="file in filteredFiles" :key="file.name" class="file-row">
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
              <IconButton
                v-if="tab === 'mods' || tab === 'resourcepacks' || tab === 'shaderpacks'"
                :icon="file.enabled ? 'check' : 'close'"
                :label="file.enabled ? '禁用' : '启用'"
                variant="ghost"
                :size="30"
                @click="toggleFile(file)"
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
        <WoodButton variant="ghost" size="sm" @click="repairOpen = false">关闭</WoodButton>
        <WoodButton variant="soft" size="sm" @click="openVersionFolder">
          <AppIcon name="folder" :size="13" />
          打开版本目录
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
                  两种格式都会把所选内容直接打包进整合包，导入时无需联网下载。
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
  border-radius: 6px;
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
  gap: 10px;
}

.ins-loading .skeleton {
  width: 140px;
  height: 12px;
}

.ins-layout {
  display: grid;
  grid-template-columns: 190px minmax(0, 1fr);
  gap: 16px;
  align-items: start;
}

.ins-side {
  padding: 12px 10px;
  display: flex;
  flex-direction: column;
  gap: 3px;
  position: sticky;
  top: 0;
}

.ins-side__item {
  display: flex;
  align-items: center;
  gap: 9px;
  height: 38px;
  padding: 0 11px;
  border-radius: 4px;
  color: var(--ink-soft);
  font-size: 13px;
  font-weight: 600;
  text-align: left;
  transition: background var(--dur-2) ease, color var(--dur-2) ease;
}

.ins-side__item:hover {
  background: rgba(var(--accent-rgb), 0.06);
  color: var(--walnut-deep);
}

.ins-side__item--active {
  background: var(--panel);
  color: var(--walnut-deep);
  box-shadow: var(--shadow-1);
}

.ins-side__item--active .icon {
  color: var(--moss);
}

.ins-side__label {
  flex: 1;
}

.ins-side__count {
  font-size: 10.5px;
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
  font-size: 10px;
  letter-spacing: 0.14em;
  color: var(--muted);
  text-transform: uppercase;
}

.ins-side__path {
  font-size: 10.5px;
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
  gap: 16px;
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
  gap: 12px;
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
  gap: 18px;
  padding: 10px 0;
  border-bottom: 1px dashed var(--line);
}

.setting--last {
  border-bottom: none;
}

/* 长参数输入改为上下布局，避免和标签互相挤压 */
.setting--stack {
  flex-direction: column;
  align-items: stretch;
  gap: 8px;
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
  font-size: 13px;
  color: var(--walnut-deep);
}

.setting__label span {
  font-size: 11px;
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
  border-radius: 4px;
  border: 1px solid var(--line-strong);
  background: var(--panel);
  color: var(--ink);
  font-size: 13px;
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
  gap: 8px;
}

.icon-chip {
  display: flex;
  align-items: center;
  gap: 7px;
  height: 36px;
  padding: 0 10px 0 6px;
  border-radius: 4px;
  border: 1px solid var(--line);
  background: var(--panel);
  color: var(--ink-soft);
  font-size: 12px;
  transition: border-color var(--dur-2) ease, color var(--dur-2) ease;
}

.icon-chip:hover {
  border-color: var(--walnut-light);
}

.icon-chip--active {
  border-color: var(--moss);
  color: var(--walnut-deep);
  box-shadow: 0 0 0 3px rgba(var(--accent-rgb), 0.08);
}

.icon-chip__tile {
  width: 24px;
  height: 24px;
  border-radius: 6px;
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
  border-radius: 5px;
  object-fit: cover;
}

.quick-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
  gap: 8px;
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
  font-size: 13px;
  color: var(--ink);
}

.count-strip {
  display: grid;
  grid-template-columns: repeat(5, minmax(0, 1fr));
  gap: 6px;
  margin-top: 12px;
}

.count {
  display: flex;
  flex-direction: column;
  gap: 1px;
  padding: 8px 10px;
  border-radius: 4px;
  text-align: left;
  transition: background var(--dur-2) ease;
}

.count:hover {
  background: rgba(var(--accent-rgb), 0.06);
}

.count__num {
  font-size: 24px;
  line-height: 1.05;
  color: var(--walnut-deep);
}

.count__label {
  font-size: 10.5px;
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
  gap: 10px;
  margin: 4px 0;
}

.world-pick {
  display: flex;
  align-items: center;
  gap: 10px;
  margin: 10px 0 4px;
  padding: 10px 12px;
  border: 1px dashed var(--line);
  border-radius: 4px;
  background: rgba(var(--accent-rgb), 0.03);
}

.proj-tip {
  display: flex;
  align-items: center;
  gap: 10px;
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
  font-size: 12px;
  color: var(--ink);
}

.proj-tip__status {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 11px;
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
  gap: 10px;
  padding: 10px 0;
}

.seed-body {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.seed-world {
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.seed-world strong {
  font-size: 14px;
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
  gap: 10px;
  padding: 10px 0;
}

.check-body {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.check-ok {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 0;
  font-size: 13px;
  color: var(--ink);
}

.repair-err-title {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 4px 0 0;
  font-size: 12px;
  color: #b4483c;
}

.check-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin: 0;
  padding: 0;
  list-style: none;
  max-height: 260px;
  overflow-y: auto;
}

.check-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 10px;
  border-radius: 4px;
  border: 1px solid var(--line);
  background: var(--panel-2);
  font-size: 12px;
}

.check-tag {
  flex-shrink: 0;
  padding: 1px 6px;
  border-radius: 3px;
  border: 1px solid var(--line);
  color: var(--ink-soft);
  font-size: 11px;
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
  font-size: 12px;
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
  gap: 16px;
  padding: 24px 10px;
  color: var(--walnut);
}

.file-empty__title {
  font-size: 14px;
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

.file-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 9px;
  border-radius: 4px;
  transition: background var(--dur-1) ease;
}

.file-row:hover {
  background: rgba(var(--accent-rgb), 0.05);
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
  gap: 12px;
  padding-top: 4px;
}

.shot-card {
  display: flex;
  flex-direction: column;
  border: 1px solid var(--line);
  border-radius: 5px;
  overflow: hidden;
  background: var(--panel-2);
  transition: border-color var(--dur-2) ease, transform var(--dur-2) var(--ease-out-expo),
    box-shadow var(--dur-2) ease;
}

.shot-card:hover {
  border-color: var(--line-strong);
  transform: translateY(-2px);
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
  gap: 8px;
  padding: 6px 8px 6px 10px;
}

.shot-card__name {
  flex: 1;
  min-width: 0;
  font-size: 11.5px;
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
  font-size: 13px;
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
  font-size: 10.5px;
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
  border-radius: 6px;
  border: 1px solid var(--line-strong);
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
  gap: 12px;
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
  gap: 12px;
}

.pack-field {
  display: flex;
  flex-direction: column;
  gap: 5px;
}

.pack-field__label {
  font-size: 11.5px;
  color: var(--muted);
  letter-spacing: 0.04em;
}

.export-two {
  display: grid;
  grid-template-columns: minmax(0, 1.4fr) minmax(0, 1fr);
  gap: 12px;
}

.export-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-top: 2px;
}

.export-link {
  font-size: 11.5px;
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
  padding: 8px;
  border: 1px dashed var(--line-strong);
  border-radius: 4px;
}

.export-group--custom {
  cursor: default;
}

.export-extra-add {
  display: flex;
  gap: 6px;
  margin-top: -6px;
}

.pack-field__hint {
  margin-top: 4px;
  font-size: 11px;
  line-height: 1.5;
}

.export-group {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 9px;
  border: 1px solid var(--line);
  border-radius: 4px;
  background: var(--panel);
  text-align: left;
  color: var(--ink-soft);
  transition: border-color var(--dur-2) ease;
}

.export-group:hover {
  border-color: var(--walnut-light);
}

.export-group--on {
  border-color: var(--moss);
  color: var(--walnut-deep);
}

.export-group__box {
  width: 15px;
  height: 15px;
  border-radius: 4px;
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
  font-size: 12.5px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.export-group__meta {
  font-size: 10px;
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
  transition: opacity 180ms ease;
}

.pack-modal-enter-active .pack-modal__card,
.pack-modal-leave-active .pack-modal__card {
  transition: transform 220ms var(--ease-out-expo);
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
  font-size: 12px;
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
  gap: 10px;
  padding: 12px 14px;
  border-radius: 6px;
  border: 1px solid var(--line-strong);
  background: linear-gradient(180deg, rgba(var(--accent-rgb), 0.06), transparent);
}

.update-panel__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}

.update-panel__title {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.update-panel__title strong {
  font-size: 13px;
  font-weight: 700;
  color: var(--walnut-deep);
}

.update-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.update-row {
  display: flex;
  align-items: center;
  gap: 10px;
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
  font-size: 12.5px;
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
  font-size: 10px;
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
  font-size: 11.5px;
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
  gap: 6px;
}

.opts-presets__head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}

.opts-presets__head strong {
  font-size: 13px;
  color: var(--walnut-deep);
}

.opts-preset {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 9px 11px;
  border: 1px solid var(--line);
  border-radius: 5px;
  background: var(--panel-2);
}

.opts-preset__info {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.opts-preset__info strong {
  font-size: 12.5px;
  color: var(--ink);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
