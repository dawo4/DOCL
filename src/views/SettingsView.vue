<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { listen } from "@tauri-apps/api/event";
import { confirm } from "@tauri-apps/plugin-dialog";
import {
  api,
  openFolder,
  pickDirectory,
  pickImage,
  pickJavaExe,
  pickSettingsFile,
  saveSettingsDialog,
  type Config,
  type GameDirEntry,
  type SourceProbe,
  type UpdateInfo,
  type UpdateProgress,
} from "../api";
import defaultProfileCard from "../assets/profile-card.jpg";
import defaultProfileHero from "../assets/profile-hero.jpg";
import AppIcon from "../components/AppIcon.vue";
import IconButton from "../components/IconButton.vue";
import WoodButton from "../components/WoodButton.vue";
import WoodInput from "../components/WoodInput.vue";
import WoodSelect from "../components/WoodSelect.vue";
import WoodSlider from "../components/WoodSlider.vue";
import WoodToggle from "../components/WoodToggle.vue";
import {
  applyTheme,
  installJava,
  loadBackground,
  notify,
  refreshConfig,
  refreshInstanceMetas,
  refreshInstalled,
  refreshJava,
  refreshVersions,
  saveConfig,
  shuffleBackground,
  startupUpdate,
  store,
} from "../store";

const form = reactive<Config>({
  game_dir: "",
  game_dirs: [],
  recent_servers: [],
  hall_url: "",
  terracotta_nodes: "",
  update_url: "",
  update_mode: "notify",
  java_path: null,
  java_install_dir: null,
  memory_mb: 4096,
  memory_auto: true,
  memory_mode: "auto",
  java_args: "",
  game_args: "",
  pre_launch: "",
  disable_launch_wrapper: false,
  disable_lwjgl_unsafe: false,
  prefer_gpu: false,
  window_width: 1280,
  window_height: 720,
  fullscreen: false,
  download_source: "auto",
  concurrency: 16,
  speed_limit: 0,
  animations: true,
  version_isolation: true,
  selected_version: null,
  selected_account: null,
  theme: "paper",
  background_image: null,
  background_dir: null,
  background_fit: "cover",
  background_blur: 18,
  glass_opacity: 78,
  glass_blur: 14,
  background_dim: 0,
  profile_hero_image: null,
  profile_card_image: null,
  profile_text_color: null,
  ui_radius: "default",
  ui_font: "default",
  ui_density: "normal",
  window_vibrancy: false,
  mcmod_names: true,
  remember_window: true,
  launcher_window_w: 0,
  launcher_window_h: 0,
});

/// 从 store 同步到表单期间要抑制自动保存，避免刚打开页面就写盘。
let syncing = false;

watch(
  () => store.config,
  async (config) => {
    if (!config) return;
    syncing = true;
    Object.assign(form, config);
    await nextTick();
    syncing = false;
  },
  { immediate: true }
);

/// 所有设置都即时保存
const autoSaveKeys: (keyof Config)[] = [
  "game_dir",
  "theme",
  "background_image",
  "background_dir",
  "background_fit",
  "background_blur",
  "glass_blur",
  "glass_opacity",
  "background_dim",
  "profile_hero_image",
  "profile_card_image",
  "profile_text_color",
  "ui_radius",
  "ui_font",
  "ui_density",
  "animations",
  "hall_url",
  "terracotta_nodes",
  "update_url",
  "update_mode",
  "java_path",
  "java_install_dir",
  "memory_mb",
  "memory_auto",
  "memory_mode",
  "java_args",
  "game_args",
  "pre_launch",
  "disable_launch_wrapper",
  "disable_lwjgl_unsafe",
  "prefer_gpu",
  "window_width",
  "window_height",
  "fullscreen",
  "download_source",
  "concurrency",
  "speed_limit",
  "version_isolation",
  "window_vibrancy",
  "mcmod_names",
  "remember_window",
];

let autoTimer: number | undefined;
let autoSeq = 0;
let autoPending = false;
/// 设置页打开期间定时刷新内存占用
let memoryTimer: number | undefined;

async function refreshMemory() {
  try {
    store.systemMemory = await api.systemMemory();
  } catch {
    // 读取失败保留上一次数值，不打扰用户
  }
}

function autoPatch(): Partial<Config> {
  const patch: Partial<Config> = {};
  for (const key of autoSaveKeys) {
    // 游戏目录清空时保留上一次的有效路径，避免把启动器保存成无目录状态
    if (key === "game_dir" && !form.game_dir.trim()) continue;
    (patch as Record<string, unknown>)[key] = form[key];
  }
  return patch;
}

async function runAutoSave() {
  if (!store.config) return;
  autoPending = false;
  const seq = autoSeq;
  const previousDir = store.config.game_dir;
  try {
    const saved = await api.saveConfig({ ...store.config, ...autoPatch() });
    // 保存期间又改过设置就放弃回填，让下一次防抖保存继续写入新值
    if (seq === autoSeq) {
      syncing = true;
      store.config = saved;
      // 后端在系统不支持时会回滚毛玻璃开关，这里把表单同步回去并提示
      if (!saved.window_vibrancy && form.window_vibrancy) {
        form.window_vibrancy = false;
        notify("当前系统不支持毛玻璃透出，已自动关闭");
      }
      await nextTick();
      syncing = false;
      if (saved.game_dir !== previousDir) {
        await loadDirs();
        await refreshInstalled();
        notify(`游戏目录已切换为 ${saved.game_dir}`);
      }
    }
  } catch (err) {
    notify(`自动保存失败：${String(err)}`, "err");
  }
}

watch(
  () => autoSaveKeys.map((key) => form[key]),
  () => {
    if (syncing) return;
    autoSeq += 1;
    autoPending = true;
    window.clearTimeout(autoTimer);
    autoTimer = window.setTimeout(() => void runAutoSave(), 600);
  }
);

const runtimeMajor = ref("21");
const runtimeBusy = ref(false);

type SettingsTab =
  | "game"
  | "java"
  | "appearance"
  | "performance"
  | "download"
  | "multiplayer"
  | "update";
const settingsTab = ref<SettingsTab>("game");

const settingsTabs: { value: SettingsTab; label: string; icon: string }[] = [
  { value: "game", label: "游戏与启动", icon: "cube" },
  { value: "java", label: "Java 运行时", icon: "database" },
  { value: "appearance", label: "外观", icon: "image" },
  { value: "performance", label: "性能与参数", icon: "sun" },
  { value: "download", label: "下载", icon: "download" },
  { value: "multiplayer", label: "联机", icon: "globe" },
  { value: "update", label: "更新", icon: "refresh" },
];

async function selectTab(tab: SettingsTab) {
  settingsTab.value = tab;
  await nextTick();
  document.querySelector(".main")?.scrollTo({ top: 0, behavior: "smooth" });
}

const themes = [
  { value: "paper", label: "纸木", colors: ["#f4ebdd", "#6f4224", "#5e7b49"] },
  { value: "celadon", label: "青瓷", colors: ["#eef1e8", "#3f6b57", "#7a9b6d"] },
  { value: "terracotta", label: "赤陶", colors: ["#f6e9dc", "#9b4f3a", "#7d8455"] },
  { value: "moon", label: "月白", colors: ["#eef1f6", "#4b5d78", "#5f8892"] },
  { value: "sakura", label: "樱花", colors: ["#fdf1f2", "#b0566e", "#d99aab"] },
  { value: "ocean", label: "深海", colors: ["#eaf2f6", "#2f6076", "#3e8c8c"] },
  { value: "lavender", label: "香芋", colors: ["#f3eff8", "#6d5590", "#b49ccb"] },
  { value: "nocturne", label: "夜墨", colors: ["#1d1a16", "#c08a55", "#88a867"] },
  { value: "graphite", label: "石墨", colors: ["#1b1d20", "#9fb4c7", "#8aa88a"] },
  { value: "forge", label: "硬朗", colors: ["#0d1217", "#eef3f7", "#6fae67"] },
  { value: "ink", label: "墨染", colors: ["#f2f1ec", "#2c2c2c", "#b23a2e"] },
  { value: "neon", label: "霓虹", colors: ["#0b0a14", "#22d3ee", "#f472b6"] },
  { value: "wheat", label: "麦田", colors: ["#f8f1dd", "#7a5a26", "#d0a12c"] },
  { value: "snow", label: "雪原", colors: ["#f1f5f8", "#3b5f7f", "#7fb3c8"] },
];

const builtinBackgrounds = [
  { value: "forest", label: "林间晨雾" },
  { value: "dune", label: "赤陶沙丘" },
  { value: "night", label: "夜航星空" },
  { value: "sakura", label: "樱花季" },
  { value: "ocean", label: "深海波光" },
  { value: "aurora", label: "极光" },
  { value: "parchment", label: "宣纸纹" },
  { value: "porcelain", label: "青花瓷" },
  { value: "cloud", label: "云海" },
  { value: "mist", label: "薄雾" },
  { value: "grid", label: "石墨网格" },
  { value: "neon", label: "霓虹街区" },
  { value: "wheat", label: "麦浪" },
  { value: "snow", label: "雪原" },
];

const radiusOptions = [
  { value: "sharp", label: "硬朗直角" },
  { value: "default", label: "标准圆角" },
  { value: "round", label: "柔和圆润" },
];

const densityOptions = [
  { value: "compact", label: "紧凑" },
  { value: "normal", label: "标准" },
  { value: "comfy", label: "宽松" },
];

/// 字体预览直接使用对应字体栈，点选后立即应用到整个界面
const fontOptions = [
  {
    value: "default",
    label: "默认",
    stack: '"Microsoft YaHei UI", "Microsoft YaHei", "PingFang SC", sans-serif',
  },
  {
    value: "yahei",
    label: "黑体",
    stack: '"Microsoft YaHei", "PingFang SC", "Noto Sans SC", sans-serif',
  },
  {
    value: "songti",
    label: "宋体",
    stack: '"SimSun", "Songti SC", "Noto Serif SC", serif',
  },
  {
    value: "kaiti",
    label: "楷体",
    stack: '"KaiTi", "STKaiti", "楷体", serif',
  },
];

const memoryModes = [
  { value: "auto", label: "自动（G1GC）" },
  { value: "g1gc", label: "调优 G1GC" },
  { value: "zgc", label: "ZGC（低暂停）" },
  { value: "custom", label: "自定义（不追加 GC 参数）" },
];

const totalMb = computed(() => store.systemMemory.total_mb || 0);
const usedMb = computed(() =>
  Math.max(0, totalMb.value - (store.systemMemory.available_mb || 0))
);
/// 推荐值由后端按「当前可用内存」实时计算，设置页定时刷新后随之波动
const recommendedMb = computed(() => store.systemMemory.recommended_mb || 4096);
const sliderMax = computed(() =>
  totalMb.value
    ? Math.min(32768, Math.max(2048, Math.floor((totalMb.value - 1024) / 512) * 512))
    : 16384
);
const effectiveMemory = computed(() =>
  form.memory_auto ? recommendedMb.value : form.memory_mb
);
const memoryGb = computed(() => (effectiveMemory.value / 1024).toFixed(1));
const usedPercent = computed(() =>
  totalMb.value ? Math.min(100, (usedMb.value / totalMb.value) * 100) : 0
);

function memoryLabel(mb: number) {
  return (mb / 1024).toFixed(1);
}

function setMemoryAuto() {
  form.memory_auto = true;
  form.memory_mb = recommendedMb.value;
}

function setMemoryManual() {
  form.memory_auto = false;
  if (form.memory_mb < 1024) form.memory_mb = recommendedMb.value;
}

watch(
  () => [form.memory_auto, recommendedMb.value],
  () => {
    if (form.memory_auto) form.memory_mb = recommendedMb.value;
  },
  { immediate: true }
);

watch(
  () => form.theme,
  (theme) => {
    if (theme) applyTheme(theme);
  }
);

watch(
  () => [form.background_blur, form.glass_blur, form.glass_opacity],
  () => {
    if (!store.config) return;
    store.config.background_blur = form.background_blur;
    store.config.glass_blur = form.glass_blur;
    store.config.glass_opacity = form.glass_opacity;
  }
);

watch(
  () => [form.background_dim, form.ui_radius, form.ui_font, form.ui_density],
  () => {
    if (!store.config) return;
    store.config.background_dim = form.background_dim;
    store.config.ui_radius = form.ui_radius;
    store.config.ui_font = form.ui_font;
    store.config.ui_density = form.ui_density;
  }
);

async function chooseBackground() {
  const path = await pickImage();
  if (!path) return;
  try {
    const info = await api.importBackground(path);
    store.backgroundPreset = "";
    store.backgroundUrl = info.data_url;
    // 单图与文件夹互斥，设置单图时清掉文件夹
    form.background_dir = null;
    form.background_image = info.path;
    await runAutoSave();
    notify("背景已应用并自动保存");
  } catch (err) {
    notify(`设置背景失败：${String(err)}`, "err");
  }
}

async function chooseBackgroundDir() {
  const dir = await pickDirectory("选择背景图文件夹");
  if (!dir) return;
  form.background_dir = dir;
  form.background_image = null;
  await runAutoSave();
  await loadBackground();
  notify("背景文件夹已应用，每次启动随机一张");
}

function previewBuiltin(name: string) {
  store.backgroundPreset = name;
  store.backgroundUrl = "";
  form.background_dir = null;
  form.background_image = `builtin:${name}`;
  // 旧默认值偏保守，首次选择内置底图时切到更能看出磨砂质感的一组参数
  if (form.background_blur === 18) form.background_blur = 4;
  if (form.glass_blur === 14) form.glass_blur = 18;
  if (form.glass_opacity === 78) form.glass_opacity = 62;
}

function clearBackground() {
  store.backgroundUrl = "";
  store.backgroundPreset = "";
  form.background_dir = null;
  form.background_image = null;
  notify("已移除背景");
}

/* ---------- 版本页卡片底图 ---------- */

const profileHeroPreview = ref(defaultProfileHero);
const profileCardPreview = ref(defaultProfileCard);
let previewSeq = 0;

async function loadProfilePreviews() {
  const seq = (previewSeq += 1);
  let hero = defaultProfileHero;
  let card = defaultProfileCard;
  if (form.profile_hero_image) {
    try {
      hero = await api.backgroundDataUrl(form.profile_hero_image);
    } catch {
      // 文件被移动/删除时保留内置图，不打断设置页
    }
  }
  if (form.profile_card_image) {
    try {
      card = await api.backgroundDataUrl(form.profile_card_image);
    } catch {
      // 同上
    }
  }
  // 连续切换图片时只采用最后一次结果，避免旧请求覆盖新预览
  if (seq !== previewSeq) return;
  profileHeroPreview.value = hero;
  profileCardPreview.value = card;
}

watch(
  () => [form.profile_hero_image, form.profile_card_image],
  () => void loadProfilePreviews(),
  { immediate: true }
);

async function chooseProfileCardImage(kind: "hero" | "card") {
  const path = await pickImage();
  if (!path) return;
  if (kind === "hero") {
    form.profile_hero_image = path;
    notify("游玩档案底图已更新");
  } else {
    form.profile_card_image = path;
    notify("最常玩版本底图已更新");
  }
  try {
    const url = await api.backgroundDataUrl(path);
    if (kind === "hero") profileHeroPreview.value = url;
    else profileCardPreview.value = url;
  } catch {
    // 保存后由 loadProfilePreviews 再次尝试
  }
}

function resetProfileCardImage(kind: "hero" | "card") {
  if (kind === "hero") {
    form.profile_hero_image = null;
    profileHeroPreview.value = defaultProfileHero;
    notify("已恢复游玩档案内置底图");
  } else {
    form.profile_card_image = null;
    profileCardPreview.value = defaultProfileCard;
    notify("已恢复最常玩版本内置底图");
  }
}

/// 版本页卡片文字颜色，空表示跟随主题
function setProfileTextColor(event: Event) {
  form.profile_text_color = (event.target as HTMLInputElement).value;
}

function resetProfileTextColor() {
  form.profile_text_color = null;
  notify("卡片字体颜色已恢复默认");
}

/// 背景适应方式选项（cover 为默认铺满裁剪）
const fitOptions = [
  { value: "cover", label: "铺满裁剪" },
  { value: "contain", label: "完整显示" },
  { value: "stretch", label: "拉伸铺满" },
  { value: "center", label: "原始大小" },
];

/// 内置底图的缩略渐变，用于背景预览（与实际壁纸同色系）
const presetPreview: Record<string, string> = {
  forest: "linear-gradient(180deg, #b7ccb0 0 40%, #7d9a6b 41% 60%, #45603c 61%)",
  dune: "linear-gradient(180deg, #f7dfc0 0 40%, #d9a97f 41% 60%, #a8623f 61%)",
  night: "linear-gradient(180deg, #141d2e 0 46%, #1e2c42 47% 72%, #16202e 73%)",
  sakura: "linear-gradient(180deg, #fdeef2 0 38%, #f8d7e1 39% 66%, #e6a3ba 67%)",
  ocean: "linear-gradient(180deg, #dff1f7 0 36%, #5f9cb2 37% 66%, #2f6076 67%)",
  aurora:
    "radial-gradient(circle at 30% 25%, #78eabe 0 18%, transparent 40%), radial-gradient(circle at 70% 45%, #9682f0 0 14%, transparent 36%), linear-gradient(180deg, #101a2c 0 46%, #172a42 47% 72%, #101a2a 73%)",
  parchment:
    "radial-gradient(ellipse at 20% 16%, rgba(120,94,62,0.28), transparent 65%), linear-gradient(180deg, #f3ebd9 0 40%, #e9dcc3 41% 62%, #dbc8a8 63%)",
  porcelain:
    "radial-gradient(circle at 26% 22%, rgba(63,107,135,0.32), transparent 52%), radial-gradient(circle at 76% 68%, rgba(63,107,135,0.24), transparent 50%), linear-gradient(180deg, #f6faf8 0 40%, #e6f0ec 41% 66%, #d2e2dc 67%)",
  cloud:
    "radial-gradient(circle at 76% 18%, #fffcf0 0 8%, transparent 18%), radial-gradient(ellipse at 24% 70%, #ffffff 0 20%, transparent 50%), linear-gradient(180deg, #eaf1f7 0 44%, #d7e4ee 45% 68%, #bfd1df 69%)",
  mist:
    "radial-gradient(ellipse at 30% 28%, rgba(180,156,203,0.55), transparent 60%), radial-gradient(ellipse at 74% 74%, rgba(150,120,190,0.5), transparent 60%), linear-gradient(180deg, #f5f0fa 0 42%, #e7def3 43% 68%, #d5c7e6 69%)",
  grid:
    "radial-gradient(ellipse at 50% 0%, rgba(159,180,199,0.25), transparent 55%), linear-gradient(180deg, #14171a 0 46%, #1b1f24 47% 72%, #121518 73%)",
  neon:
    "radial-gradient(circle at 18% 24%, rgba(34,211,238,0.6) 0 16%, transparent 38%), radial-gradient(circle at 82% 36%, rgba(244,114,182,0.55) 0 14%, transparent 34%), linear-gradient(180deg, #0a0814 0 46%, #120f26 47% 72%, #0a0816 73%)",
  wheat:
    "radial-gradient(circle at 26% 18%, #fff4c8 0 9%, transparent 20%), radial-gradient(ellipse at 20% 108%, #b98f34 0 58%, transparent 59%), linear-gradient(180deg, #f8ecc4 0 46%, #ecd89b 47% 72%, #cda851 73%)",
  snow:
    "radial-gradient(circle at 74% 16%, #ffffff 0 8%, transparent 18%), radial-gradient(ellipse at 14% 108%, #e8f1f8 0 58%, transparent 59%), linear-gradient(180deg, #dfeefa 0 46%, #cbe1f1 47% 72%, #accbe4 73%)",
};

const hasBackground = computed(
  () => Boolean(store.backgroundUrl || store.backgroundPreset)
);

const bgPreviewStyle = computed(() => {
  const style: Record<string, string> = {};
  if (store.backgroundUrl) {
    const size =
      form.background_fit === "contain"
        ? "contain"
        : form.background_fit === "center"
          ? "auto"
          : form.background_fit === "stretch"
            ? "100% 100%"
            : "cover";
    style.backgroundImage = `url("${store.backgroundUrl}")`;
    style.backgroundSize = size;
    style.backgroundPosition = "center";
    style.backgroundRepeat = "no-repeat";
    return style;
  }
  if (store.backgroundPreset) {
    style.backgroundImage = presetPreview[store.backgroundPreset] ?? "";
    style.backgroundSize = "cover";
  }
  return style;
});

const sourceOptions = [
  { value: "auto", label: "自动（按来源智能选择）" },
  { value: "mirror", label: "仅加速镜像" },
  { value: "official", label: "仅官方源" },
];

/* ---------- 下载源测速 ---------- */
const probes = ref<SourceProbe[]>([]);
const probeBusy = ref(false);

const probeGroups = computed(() => {
  const groups = [
    { key: "mojang", label: "游戏文件与加载器" },
    { key: "mod", label: "Mod 接口（CurseForge / Modrinth）" },
  ];
  return groups
    .map((group) => ({
      ...group,
      items: probes.value
        .filter((item) => item.group === group.key)
        // 可用的排前面，同状态按延迟从低到高
        .sort((a, b) =>
          a.ok === b.ok ? a.latency_ms - b.latency_ms : a.ok ? -1 : 1
        ),
    }))
    .filter((group) => group.items.length > 0);
});

async function runProbe() {
  if (probeBusy.value) return;
  probeBusy.value = true;
  probes.value = [];
  try {
    probes.value = await api.probeDownloadSources();
  } catch (err) {
    notify(`测速失败：${String(err)}`, "err");
  } finally {
    probeBusy.value = false;
  }
}

/// 支持 Mojang 官方目录，以及常见 OpenJDK 发行版的官方下载接口
const runtimeSource = ref("mojang");
const javaSources = [
  { value: "mojang", label: "Mojang 官方" },
  { value: "temurin", label: "Eclipse Temurin" },
  { value: "zulu", label: "Azul Zulu" },
  { value: "corretto", label: "Amazon Corretto" },
];

/// 接口读取失败时按各发行版的实际支持范围兜底
const fallbackMajors: Record<string, number[]> = {
  mojang: [8, 16, 17, 21, 25],
  temurin: [8, 11, 16, 17, 21, 25],
  zulu: [8, 11, 16, 17, 21, 25],
  corretto: [8, 11, 17, 21, 25],
};

const javaOptions = ref(
  fallbackMajors.mojang.map((major) => ({ value: String(major), label: `Java ${major}` }))
);

async function loadJavaOptions(source = runtimeSource.value) {
  let majors = fallbackMajors[source] ?? [];
  try {
    const list = await api.javaRuntimeMajors(source);
    if (list.length) majors = list;
  } catch {
    // 读取失败时保留内置选项
  }
  javaOptions.value = majors.map((major) => ({
    value: String(major),
    label: `Java ${major}`,
  }));
  if (!javaOptions.value.some((option) => option.value === runtimeMajor.value)) {
    const preferred = majors.includes(21) ? 21 : majors[majors.length - 1] ?? 21;
    runtimeMajor.value = String(preferred);
  }
}

watch(runtimeSource, (source) => void loadJavaOptions(source));

const updateModes = [
  { value: "auto", label: "有新版本时自动下载" },
  { value: "notify", label: "有新版本时提示" },
  { value: "off", label: "关闭自动检查" },
];

/// 两个默认清单地址：国内加速走 jsDelivr，GitHub 原地址更新更及时。
const updateSources = [
  {
    value: "https://cdn.jsdelivr.net/gh/dawo4/DOCL@main/update.json",
    label: "国内加速（jsDelivr）",
  },
  {
    value: "https://raw.githubusercontent.com/dawo4/DOCL/main/update.json",
    label: "GitHub 原地址",
  },
  { value: "custom", label: "自定义地址" },
];

const updateSource = computed<string>({
  get: () =>
    updateSources.some((source) => source.value === form.update_url) ? form.update_url : "custom",
  set: (value) => {
    // 选自定义时保留当前地址，方便在预设基础上改
    if (value !== "custom") form.update_url = value;
  },
});

async function chooseDir() {
  const dir = await pickDirectory();
  if (dir) form.game_dir = dir;
}

async function chooseJavaDir() {
  const dir = await pickDirectory("选择 Java 安装位置");
  if (dir) form.java_install_dir = dir;
}

const gameDirs = ref<GameDirEntry[]>([]);
const activeDir = ref("");
const officialDir = ref("");
/// 目录路径 → 该目录下的版本数
const dirCounts = ref<Record<string, number>>({});

/// 截图/录屏时把 Windows 用户名替换成 Player，避免公开本机用户名
const privacy = computed(() => import.meta.env.DEV && store.maskIds);

function maskPath(path: string) {
  if (!privacy.value) return path;
  return path.replace(/([A-Za-z]:\\Users\\)[^\\/]+/i, "$1Player");
}
const dirBusy = ref(false);

onMounted(loadDirs);

const appVersion = ref("");
const portable = ref(false);
const updateInfo = ref<UpdateInfo | null>(null);
const updateBusy = ref(false);
const updateError = ref("");
const updatePath = ref("");
const updateProgress = ref<UpdateProgress | null>(null);
let stopUpdateProgress: (() => void) | null = null;

const updatePercent = computed(() => {
  const progress = updateProgress.value;
  if (!progress || !progress.total) return 0;
  return Math.min(100, Math.round((progress.downloaded / progress.total) * 100));
});

const updateProgressText = computed(() => {
  const progress = updateProgress.value;
  if (!progress) return "";
  if (progress.total) return `${updatePercent.value}%`;
  return `${(progress.downloaded / 1048576).toFixed(1)} MB`;
});

/// 便携版优先取压缩包地址；没配便携包时回退到安装包
const updateTargetUrl = computed(() =>
  portable.value && updateInfo.value?.portable_url
    ? updateInfo.value.portable_url
    : updateInfo.value?.url ?? ""
);
const isPortablePack = computed(() => /\.zip$/i.test(updateTargetUrl.value));

async function initUpdateSection() {
  try {
    appVersion.value = await api.appVersion();
  } catch {
    appVersion.value = "";
  }
  try {
    portable.value = await api.appPortable();
  } catch {
    portable.value = false;
  }
  // 启动时已经检查到的新版本直接接续展示，不用再点一次检查
  if (startupUpdate.value) {
    updateInfo.value = startupUpdate.value.info;
    updatePath.value = startupUpdate.value.path ?? "";
  }
  stopUpdateProgress = await listen<UpdateProgress>("update-progress", (event) => {
    updateProgress.value = event.payload;
  });
}

onMounted(() => {
  // 从启动失败提示跳过来时直接定位到 Java 分类
  if (store.settingsFocus === "java") {
    settingsTab.value = "java";
    store.settingsFocus = null;
  }
  void initUpdateSection();
  void refreshMemory();
  void loadJavaOptions();
  memoryTimer = window.setInterval(refreshMemory, 2000);
});

onBeforeUnmount(() => {
  window.clearTimeout(autoTimer);
  if (memoryTimer) window.clearInterval(memoryTimer);
  if (autoPending) void runAutoSave();
  stopUpdateProgress?.();
});

async function checkUpdate() {
  if (updateBusy.value) return;
  startupUpdate.value = null;
  updateBusy.value = true;
  updateError.value = "";
  updateInfo.value = null;
  updatePath.value = "";
  updateProgress.value = null;
  try {
    updateInfo.value = await api.updateCheck(form.update_url);
  } catch (err) {
    updateError.value = String(err);
  } finally {
    updateBusy.value = false;
  }
}

async function downloadUpdate() {
  if (!updateInfo.value || updateBusy.value) return;
  updateBusy.value = true;
  updateError.value = "";
  updatePath.value = "";
  updateProgress.value = { downloaded: 0, total: 0 };
  try {
    updatePath.value = await api.updateDownload(
      updateTargetUrl.value,
      updateInfo.value.latest
    );
  } catch (err) {
    updateError.value = String(err);
    updateProgress.value = null;
  } finally {
    updateBusy.value = false;
  }
}

/// 便携版原地更新：解压、替换自身并重启，数据在程序目录内不受影响
const applyingUpdate = ref(false);

async function applyPortableUpdate() {
  if (!updatePath.value || applyingUpdate.value) return;
  const ok = await confirm(
    "将关闭启动器、替换程序文件后自动重启。\n配置、账号与游戏数据都在程序目录内，不受影响。",
    { title: "安装便携更新", kind: "info" }
  );
  if (!ok) return;
  applyingUpdate.value = true;
  try {
    await api.portableUpdateInstall(updatePath.value);
  } catch (err) {
    notify(`便携更新失败：${String(err)}`, "err");
    applyingUpdate.value = false;
  }
}

async function installUpdate() {
  if (!updatePath.value) return;
  try {
    await api.updateInstall(updatePath.value);
  } catch (err) {
    updateError.value = String(err);
  }
}

async function loadDirs() {
  try {
    const view = await api.gameDirs();
    gameDirs.value = view.dirs;
    activeDir.value = view.active;
    officialDir.value = view.official;
  } catch {
    gameDirs.value = [];
  }
  await loadDirCounts();
}

/// 每个目录下的版本数量，用于在列表里显示
async function loadDirCounts() {
  try {
    dirCounts.value = await api.gameDirVersionCounts();
  } catch {
    dirCounts.value = {};
  }
}

async function adoptConfig(config: Config) {
  syncing = true;
  store.config = config;
  Object.assign(form, config);
  await nextTick();
  syncing = false;
  activeDir.value = config.game_dir;
  await refreshInstalled();
  await refreshVersions(true);
  await refreshInstanceMetas();
  await loadDirs();
  if (
    config.selected_version &&
    !store.installed.some((item) => item.id === config.selected_version)
  ) {
    await saveConfig({ selected_version: store.installed[0]?.id ?? null });
  }
}

async function switchDir(path: string) {
  if (dirBusy.value || path === activeDir.value) return;
  dirBusy.value = true;
  try {
    await adoptConfig(await api.switchGameDir(path));
    notify(`已切换到 ${path}`);
  } catch (err) {
    notify(`切换目录失败：${String(err)}`, "err");
  } finally {
    dirBusy.value = false;
  }
}

async function addDir() {
  const path = await pickDirectory();
  if (!path) return;
  dirBusy.value = true;
  try {
    await adoptConfig(await api.addGameDir(path));
    notify(`已添加并切换到 ${path}`);
  } catch (err) {
    notify(`添加目录失败：${String(err)}`, "err");
  } finally {
    dirBusy.value = false;
  }
}

async function useOfficialDir() {
  if (!officialDir.value) return;
  dirBusy.value = true;
  try {
    await adoptConfig(await api.addGameDir(officialDir.value));
    notify("已切换到官方启动器目录");
  } catch (err) {
    notify(`切换目录失败：${String(err)}`, "err");
  } finally {
    dirBusy.value = false;
  }
}

async function removeDir(path: string) {
  dirBusy.value = true;
  try {
    await adoptConfig(await api.removeGameDir(path));
    notify("已从目录列表移除，磁盘文件未删除");
  } catch (err) {
    notify(`移除目录失败：${String(err)}`, "err");
  } finally {
    dirBusy.value = false;
  }
}

async function rescanJava() {
  await refreshJava();
  notify(`检测到 ${store.javaList.length} 个 Java`);
}

const javaBusy = ref(false);

/// 手动添加自动检测不到的 Java（自定义目录、绿色版 JDK 等）
async function addManualJava() {
  if (javaBusy.value) return;
  const path = await pickJavaExe();
  if (!path) return;
  javaBusy.value = true;
  try {
    const info = await api.addJava(path);
    form.java_path = info.path;
    await refreshJava();
    await refreshConfig();
    notify(`已添加 Java ${info.major}（${info.vendor} ${info.kind}），并设为优先使用`);
  } catch (err) {
    notify(`添加 Java 失败：${String(err)}`, "err");
  } finally {
    javaBusy.value = false;
  }
}

async function installRuntime() {
  if (runtimeBusy.value) return;
  runtimeBusy.value = true;
  try {
    await installJava(Number(runtimeMajor.value), runtimeSource.value);
  } finally {
    runtimeBusy.value = false;
  }
}

const backupBusy = ref(false);
const pclBusy = ref(false);

/// 从 PCL 目录读取设置并迁移；账号缓存是加密的，只能重新登录
async function importFromPcl() {
  if (pclBusy.value) return;
  const dir = await pickDirectory("选择 PCL 所在文件夹");
  if (!dir) return;
  pclBusy.value = true;
  window.clearTimeout(autoTimer);
  autoPending = false;
  try {
    const preview = await api.pclImportPreview(dir);
    const ok = await confirm(
      `将从 ${preview.root} 导入以下设置：\n\n${preview.findings.join("\n")}\n\n只覆盖读到的项，其余保持现状。`,
      { title: "从 PCL 迁移", kind: "info" }
    );
    if (!ok) return;
    const config = await api.pclImportApply(dir);
    await adoptConfig(config);
    await loadBackground();
    notify("已导入 PCL 设置");
  } catch (err) {
    notify(`导入 PCL 设置失败：${String(err)}`, "err");
  } finally {
    pclBusy.value = false;
  }
}

async function exportSettings() {
  if (backupBusy.value) return;
  const path = await saveSettingsDialog();
  if (!path) return;
  backupBusy.value = true;
  try {
    await api.exportSettings(path);
    notify("设置已导出");
  } catch (err) {
    notify(`导出设置失败：${String(err)}`, "err");
  } finally {
    backupBusy.value = false;
  }
}

async function importSettings() {
  const path = await pickSettingsFile();
  if (!path) return;
  backupBusy.value = true;
  window.clearTimeout(autoTimer);
  autoPending = false;
  try {
    const config = await api.importSettings(path);
    await adoptConfig(config);
    await loadBackground();
    notify("设置已导入并生效");
  } catch (err) {
    notify(`导入设置失败：${String(err)}`, "err");
  } finally {
    backupBusy.value = false;
  }
}

function useAutoJava() {
  form.java_path = null;
  notify("已改为自动选择 Java");
}

/// 删除启动器下载的运行时（系统安装的 Java 不提供删除，避免误删）
async function removeJava(path: string) {
  const ok = await confirm("确定删除这个 Java 运行时？删除后需要重新下载才能使用。", {
    title: "删除 Java 运行时",
    kind: "warning",
  });
  if (!ok) return;
  try {
    await api.deleteJava(path);
    if (form.java_path === path) form.java_path = null;
    await refreshJava();
    notify("已删除该 Java 运行时");
  } catch (err) {
    notify(`删除失败：${String(err)}`, "err");
  }
}
</script>

<template>
  <div class="page">
    <div class="settings-layout">
      <aside class="settings-side card">
        <button
          v-for="tab in settingsTabs"
          :key="tab.value"
          class="settings-side__item"
          :class="{ 'settings-side__item--active': settingsTab === tab.value }"
          @click="selectTab(tab.value)"
        >
          <AppIcon :name="tab.icon" :size="16" />
          <span class="settings-side__label">{{ tab.label }}</span>
        </button>
      </aside>

      <div class="settings-main">
    <section v-if="settingsTab === 'game'" class="card card-pad group reveal">
      <div class="group-head">
        <div class="group-titles">
          <div class="section-title">游戏与启动</div>
        </div>
        <span class="group-line" />
      </div>

      <div class="setting setting--stack">
        <div class="setting__label">
          <strong>游戏目录</strong>
          <span>versions / assets / libraries 都放在这里；路径不可直接编辑</span>
        </div>
        <div class="setting__control setting__control--wide">
          <div class="dir-current">
            <span class="dir-current__path mono" :title="form.game_dir">
              {{ maskPath(form.game_dir) }}
            </span>
            <div class="dir-current__actions">
              <button class="dir-link" @click="openFolder(form.game_dir)">
                <AppIcon name="folder" :size="14" />
                在文件资源管理器中显示
              </button>
              <WoodButton variant="soft" size="sm" @click="chooseDir">更改目录</WoodButton>
            </div>
          </div>
        </div>
      </div>
      <div class="setting setting--stack">
        <div class="setting__label">
          <strong>游戏目录列表</strong>
          <span>可以保存多个 .minecraft 文件夹，随时切换</span>
        </div>
        <div class="setting__control setting__control--wide">
          <div class="dir-list">
            <div
              v-for="dir in gameDirs"
              :key="dir.path"
              class="dir-row"
              :class="{ 'dir-row--active': dir.path === activeDir }"
            >
              <button class="dir-row__main" @click="switchDir(dir.path)">
                <AppIcon name="folder" :size="15" />
                <span class="dir-row__text">
                  <strong>{{ dir.name }}</strong>
                  <span class="mono">{{ maskPath(dir.path) }}</span>
                </span>
                <span class="dir-row__count mono">{{ dirCounts[dir.path] ?? 0 }} 个版本</span>
                <span v-if="dir.path === activeDir" class="tag tag--grass">当前</span>
              </button>
              <IconButton
                icon="folder"
                label="打开目录"
                :size="28"
                @click="openFolder(dir.path)"
              />
              <IconButton
                icon="trash"
                label="从列表移除"
                :size="28"
                :disabled="gameDirs.length <= 1 || dirBusy"
                @click="removeDir(dir.path)"
              />
            </div>
          </div>
          <div class="row dir-actions">
            <WoodButton variant="soft" size="sm" :disabled="dirBusy" @click="addDir">
              <AppIcon name="plus" :size="14" />
              添加已有文件夹
            </WoodButton>
            <WoodButton
              variant="ghost"
              size="sm"
              :disabled="dirBusy || !officialDir"
              @click="useOfficialDir"
            >
              官方启动器目录
            </WoodButton>
          </div>
        </div>
      </div>
      <div class="setting">
        <div class="setting__label">
          <strong>版本隔离</strong>
          <span>每个版本使用独立的游戏目录，存档互不影响</span>
        </div>
        <div class="setting__control">
          <WoodToggle v-model="form.version_isolation" />
        </div>
      </div>
      <div class="setting">
        <div class="setting__label">
          <strong>启动窗口尺寸</strong>
          <span>游戏内分辨率，按需调整</span>
        </div>
        <div class="setting__control">
          <div class="row resolution">
            <WoodInput
              :model-value="String(form.window_width)"
              placeholder="宽"
              @update:model-value="form.window_width = Number($event) || 1280"
            />
            <span class="muted resolution__x">×</span>
            <WoodInput
              :model-value="String(form.window_height)"
              placeholder="高"
              @update:model-value="form.window_height = Number($event) || 720"
            />
          </div>
        </div>
      </div>
      <div class="setting">
        <div class="setting__label">
          <strong>从 PCL 迁移</strong>
          <span>选择 PCL 所在文件夹，读取内存、Java、窗口与下载设置，并接管它的 .minecraft；账号需重新登录</span>
        </div>
        <div class="setting__control">
          <WoodButton variant="ghost" size="sm" :disabled="pclBusy" @click="importFromPcl">
            <AppIcon name="package" :size="14" />
            {{ pclBusy ? "读取中…" : "选择 PCL 文件夹" }}
          </WoodButton>
        </div>
      </div>
      <div class="setting setting--last">
        <div class="setting__label">
          <strong>配置备份</strong>
          <span>导出为 JSON 文件，可在另一台电脑导入；不含账号、皮肤与存档</span>
        </div>
        <div class="setting__control">
          <div class="row">
            <WoodButton variant="soft" size="sm" :disabled="backupBusy" @click="exportSettings">
              导出设置
            </WoodButton>
            <WoodButton variant="ghost" size="sm" :disabled="backupBusy" @click="importSettings">
              导入设置
            </WoodButton>
          </div>
        </div>
      </div>
    </section>

    <section v-if="settingsTab === 'java'" class="card card-pad group reveal" style="--i: 1">
      <div class="group-head">
        <div class="group-titles">
          <div class="section-title">Java 运行时</div>
        </div>
        <span class="group-line" />
        <div class="row">
          <WoodButton variant="ghost" size="sm" @click="rescanJava">
            <AppIcon name="refresh" :size="14" />
            重新检测
          </WoodButton>
          <WoodButton variant="soft" size="sm" :disabled="javaBusy" @click="addManualJava">
            <AppIcon name="plus" :size="14" />
            {{ javaBusy ? "校验中…" : "手动添加" }}
          </WoodButton>
        </div>
      </div>

      <p class="muted small">
        按版本自动选择 Java；手动指定后优先使用。
      </p>

      <div class="java-list">
        <div
          v-for="java in store.javaList"
          :key="java.path"
          class="java-item"
          :class="{ 'java-item--active': form.java_path === java.path }"
          role="button"
          tabindex="0"
          @click="form.java_path = java.path"
          @keydown.enter="form.java_path = java.path"
        >
          <span class="java-item__major display">Java {{ java.major }}</span>
          <span class="java-item__path mono" :title="java.path">{{ java.path }}</span>
          <AppIcon
            v-if="form.java_path === java.path"
            name="check"
            :size="16"
            class="java-item__check"
          />
          <span class="tag">{{ java.vendor }} {{ java.kind }}</span>
          <span class="tag">{{ java.source }}</span>
          <IconButton
            v-if="java.source === '启动器运行时'"
            icon="trash"
            label="删除这个运行时"
            variant="danger-quiet"
            :size="30"
            @click.stop="removeJava(java.path)"
          />
        </div>
        <div v-if="!store.javaList.length" class="java-empty">
          未检测到 Java，可手动添加或直接下载。
        </div>
      </div>

      <div class="row java-actions">
        <WoodButton variant="ghost" size="sm" @click="useAutoJava">
          <AppIcon name="refresh" :size="14" />
          恢复自动选择
        </WoodButton>
        <span class="muted small" v-if="form.java_path">指定：{{ form.java_path }}</span>
        <span class="muted small" v-else>当前：自动选择</span>
      </div>

      <div class="divider" />

      <div class="setting setting--stack">
        <div class="setting__label">
          <strong>Java 安装位置</strong>
          <span>下载的运行时保存到这里，留空则使用启动器数据目录</span>
        </div>
        <div class="setting__control">
          <div class="path-card">
            <span
              class="path-card__text mono"
              :title="form.java_install_dir || '默认（启动器数据目录）'"
            >
              {{ form.java_install_dir || "默认（启动器数据目录）" }}
            </span>
            <div class="path-card__foot">
              <span class="muted small">
                {{ form.java_install_dir ? "自定义位置" : "默认位置" }}
              </span>
              <div class="path-card__buttons">
                <WoodButton variant="soft" size="sm" @click="chooseJavaDir">
                  选择目录
                </WoodButton>
                <IconButton
                  v-if="form.java_install_dir"
                  icon="trash"
                  label="恢复默认位置"
                  variant="danger-quiet"
                  :size="34"
                  @click="form.java_install_dir = null"
                />
              </div>
            </div>
          </div>
        </div>
      </div>

      <div class="setting setting--last">
        <div class="setting__label">
          <strong>下载 Java 运行时</strong>
          <span>官方与常见 OpenJDK 发行版，不影响系统环境</span>
        </div>
        <div class="setting__control">
          <div class="row">
            <WoodSelect v-model="runtimeSource" :options="javaSources" class="runtime-source" />
            <WoodSelect v-model="runtimeMajor" :options="javaOptions" class="runtime-select" />
            <WoodButton
              variant="soft"
              size="sm"
              class="runtime-action"
              :disabled="runtimeBusy"
              @click="installRuntime"
            >
              <AppIcon name="download" :size="14" />
              {{ runtimeBusy ? "安装中…" : "下载并安装" }}
            </WoodButton>
          </div>
        </div>
      </div>
    </section>

    <section
      v-if="settingsTab === 'appearance'"
      class="card card-pad group reveal"
      :style="{ '--i': 2 }"
    >
      <div class="group-head">
        <div class="group-titles">
          <div class="section-title">外观</div>
        </div>
        <span class="group-line" />
      </div>

      <div class="setting setting--stack">
        <div class="setting__label">
          <strong>主题配色</strong>
          <span>即时预览并自动保存</span>
        </div>
        <div class="setting__control">
          <div class="theme-cards">
            <button
              v-for="theme in themes"
              :key="theme.value"
              class="theme-card"
              :class="{ 'theme-card--active': form.theme === theme.value }"
              @click="form.theme = theme.value"
            >
              <span
                class="theme-card__preview"
                :style="{
                  '--pv-bg': theme.colors[0],
                  '--pv-main': theme.colors[1],
                  '--pv-accent': theme.colors[2],
                }"
              >
                <i class="theme-card__side" />
                <i class="theme-card__bar" />
                <i class="theme-card__line theme-card__line--long" />
                <i class="theme-card__line" />
              </span>
              <span class="theme-card__name">{{ theme.label }}</span>
            </button>
          </div>
        </div>
      </div>

      <div class="setting setting--stack">
        <div class="setting__label">
          <strong>内置底图</strong>
          <span>点选即可预览磨砂玻璃效果，无需本地图片</span>
        </div>
        <div class="setting__control">
          <div class="bg-tiles">
            <button
              v-for="bg in builtinBackgrounds"
              :key="bg.value"
              class="bg-tile"
              :class="{
                'bg-tile--active': form.background_image === `builtin:${bg.value}`,
              }"
              @click="previewBuiltin(bg.value)"
            >
              <span
                class="bg-tile__thumb"
                :class="`bg-chip__swatch--${bg.value}`"
              />
              <span class="bg-tile__label">{{ bg.label }}</span>
            </button>
            <button
              class="bg-tile"
              :class="{ 'bg-tile--active': !form.background_image && !form.background_dir }"
              @click="clearBackground"
            >
              <span class="bg-tile__thumb bg-chip__swatch--none" />
              <span class="bg-tile__label">无背景</span>
            </button>
          </div>
        </div>
      </div>

      <div class="setting setting--stack">
        <div class="setting__label">
          <strong>背景图片</strong>
          <span>PNG / JPG / WEBP / BMP / GIF，上限 24 MB；文件夹优先于单图</span>
        </div>
        <div class="setting__control">
          <div
            class="bg-preview"
            :class="{ 'bg-preview--empty': !hasBackground }"
            :style="bgPreviewStyle"
          >
            <span v-if="!hasBackground" class="bg-preview__empty">
              还没有背景，选择一张图片或下方内置底图
            </span>
            <button class="bg-preview__pick" @click="chooseBackground">选择图片</button>
          </div>
          <div class="bg-preview__actions">
            <span class="muted small">
              {{ form.background_dir ? "使用文件夹随机图" : hasBackground ? "已设置背景" : "未设置背景" }}
            </span>
            <div class="row">
              <IconButton icon="image" label="选择背景图片" :size="34" @click="chooseBackground" />
              <IconButton icon="folder" label="选择背景文件夹" :size="34" @click="chooseBackgroundDir" />
              <IconButton
                icon="refresh"
                label="换一张"
                :size="34"
                :disabled="!form.background_dir"
                @click="shuffleBackground"
              />
              <IconButton
                icon="trash"
                label="移除背景"
                variant="danger-quiet"
                :size="34"
                :disabled="!hasBackground && !form.background_dir"
                @click="clearBackground"
              />
            </div>
          </div>
        </div>
      </div>

      <div class="setting setting--stack">
        <div class="setting__label">
          <strong>版本页卡片底图</strong>
          <span>分别使用你自己的图片；不设置时沿用内置风景图</span>
        </div>
        <div class="setting__control">
          <div class="profile-bg-grid">
            <div class="profile-bg-card">
              <span
                class="profile-bg-card__preview"
                :style="{ backgroundImage: `url(${profileHeroPreview})` }"
              />
              <div class="profile-bg-card__body">
                <strong>游玩档案</strong>
                <span>{{ form.profile_hero_image ? "自定义底图" : "内置森林落日" }}</span>
              </div>
              <div class="profile-bg-card__actions">
                <WoodButton variant="soft" size="sm" @click="chooseProfileCardImage('hero')">
                  选择图片
                </WoodButton>
                <WoodButton
                  variant="ghost"
                  size="sm"
                  :disabled="!form.profile_hero_image"
                  @click="resetProfileCardImage('hero')"
                >
                  恢复默认
                </WoodButton>
              </div>
            </div>

            <div class="profile-bg-card">
              <span
                class="profile-bg-card__preview"
                :style="{ backgroundImage: `url(${profileCardPreview})` }"
              />
              <div class="profile-bg-card__body">
                <strong>最常玩版本</strong>
                <span>{{ form.profile_card_image ? "自定义底图" : "内置林间小屋" }}</span>
              </div>
              <div class="profile-bg-card__actions">
                <WoodButton variant="soft" size="sm" @click="chooseProfileCardImage('card')">
                  选择图片
                </WoodButton>
                <WoodButton
                  variant="ghost"
                  size="sm"
                  :disabled="!form.profile_card_image"
                  @click="resetProfileCardImage('card')"
                >
                  恢复默认
                </WoodButton>
              </div>
            </div>
          </div>
        </div>
      </div>

      <div class="setting">
        <div class="setting__label">
          <strong>卡片字体颜色</strong>
          <span>底图与文字对比度不够时调整，留空使用默认浅色</span>
        </div>
        <div class="setting__control">
          <div class="row">
            <input
              class="color-well"
              type="color"
              :value="form.profile_text_color || '#fff4e2'"
              @input="setProfileTextColor"
            />
            <span class="mono small">
              {{ form.profile_text_color || "默认浅色" }}
            </span>
            <WoodButton
              variant="ghost"
              size="sm"
              :disabled="!form.profile_text_color"
              @click="resetProfileTextColor"
            >
              恢复默认
            </WoodButton>
          </div>
        </div>
      </div>

      <div class="setting">
        <div class="setting__label">
          <strong>背景适应</strong>
          <span>图片与窗口的适配方式</span>
        </div>
        <div class="setting__control">
          <div class="bg-presets">
            <button
              v-for="fit in fitOptions"
              :key="fit.value"
              class="bg-chip"
              :class="{ 'bg-chip--active': (form.background_fit || 'cover') === fit.value }"
              @click="form.background_fit = fit.value"
            >
              {{ fit.label }}
            </button>
          </div>
        </div>
      </div>

      <div class="setting">
        <div class="setting__label">
          <strong>背景模糊</strong>
          <span>{{ form.background_blur }} px，越大越柔和</span>
        </div>
        <div class="setting__control setting__control--wide">
          <WoodSlider v-model="form.background_blur" :min="0" :max="30" :step="1" />
        </div>
      </div>

      <div class="setting">
        <div class="setting__label">
          <strong>玻璃模糊</strong>
          <span>{{ form.glass_blur }} px，磨砂面板的雾化程度</span>
        </div>
        <div class="setting__control setting__control--wide">
          <WoodSlider v-model="form.glass_blur" :min="0" :max="24" :step="1" />
        </div>
      </div>

      <div class="setting">
        <div class="setting__label">
          <strong>玻璃不透明度</strong>
          <span>{{ form.glass_opacity }}%，越低越透出背景</span>
        </div>
        <div class="setting__control setting__control--wide">
          <WoodSlider v-model="form.glass_opacity" :min="30" :max="95" :step="1" />
        </div>
      </div>

      <div class="setting">
        <div class="setting__label">
          <strong>背景遮罩</strong>
          <span>{{ form.background_dim }}%，压暗底图让内容更清晰</span>
        </div>
        <div class="setting__control setting__control--wide">
          <WoodSlider v-model="form.background_dim" :min="0" :max="60" :step="2" />
        </div>
      </div>

      <div class="setting setting--stack">
        <div class="setting__label">
          <strong>窗口材质</strong>
          <span>毛玻璃会透出桌面实时画面；Win10 上拖动窗口时系统会临时降级</span>
        </div>
        <div class="setting__control">
          <div class="option-cards">
            <button
              class="option-card"
              :class="{ 'option-card--active': !form.window_vibrancy }"
              @click="form.window_vibrancy = false"
            >
              <span class="option-card__preview option-card__preview--solid" />
              <span class="option-card__name">实心窗口</span>
            </button>
            <button
              class="option-card"
              :class="{ 'option-card--active': form.window_vibrancy }"
              @click="form.window_vibrancy = true"
            >
              <span class="option-card__preview option-card__preview--glass" />
              <span class="option-card__name">毛玻璃</span>
            </button>
          </div>
        </div>
      </div>

      <div class="setting setting--stack">
        <div class="setting__label">
          <strong>界面字体</strong>
          <span>标题与正文统一切换，点击后立即生效</span>
        </div>
        <div class="setting__control">
          <div class="option-cards">
            <button
              v-for="opt in fontOptions"
              :key="opt.value"
              class="option-card"
              :class="{ 'option-card--active': form.ui_font === opt.value }"
              @click="form.ui_font = opt.value"
            >
              <span
                class="option-card__preview option-card__preview--font"
                :style="{ fontFamily: opt.stack }"
              >
                Aa 字体
              </span>
              <span class="option-card__name">{{ opt.label }}</span>
            </button>
          </div>
        </div>
      </div>

      <div class="setting setting--stack">
        <div class="setting__label">
          <strong>界面密度</strong>
          <span>控制间距与留白，小窗口可用紧凑</span>
        </div>
        <div class="setting__control">
          <div class="option-cards">
            <button
              v-for="opt in densityOptions"
              :key="opt.value"
              class="option-card"
              :class="{ 'option-card--active': form.ui_density === opt.value }"
              @click="form.ui_density = opt.value"
            >
              <span
                class="option-card__preview option-card__preview--density"
                :class="`density--${opt.value}`"
              >
                <i /><i /><i />
              </span>
              <span class="option-card__name">{{ opt.label }}</span>
            </button>
          </div>
        </div>
      </div>

      <div class="setting setting--stack">
        <div class="setting__label">
          <strong>界面圆角</strong>
          <span>卡片与面板的边角风格</span>
        </div>
        <div class="setting__control">
          <div class="option-cards">
            <button
              v-for="opt in radiusOptions"
              :key="opt.value"
              class="option-card"
              :class="{ 'option-card--active': form.ui_radius === opt.value }"
              @click="form.ui_radius = opt.value"
            >
              <span
                class="option-card__preview option-card__preview--radius"
                :class="`radius--${opt.value}`"
              />
              <span class="option-card__name">{{ opt.label }}</span>
            </button>
          </div>
        </div>
      </div>

      <div class="setting">
        <div class="setting__label">
          <strong>记住窗口大小</strong>
          <span>关闭启动器时记录当前尺寸，下次打开自动恢复；关闭开关则每次都使用默认大小</span>
        </div>
        <div class="setting__control">
          <WoodToggle v-model="form.remember_window" />
        </div>
      </div>

      <div class="setting setting--last">
        <div class="setting__label">
          <strong>界面动效</strong>
          <span>关闭后页面切换与控件过渡立即完成</span>
        </div>
        <div class="setting__control">
          <WoodToggle v-model="form.animations" />
        </div>
      </div>
    </section>

    <section
      v-if="settingsTab === 'performance'"
      class="card card-pad group reveal"
      :style="{ '--i': 3 }"
    >
      <div class="group-head">
        <div class="group-titles">
          <div class="section-title">性能与参数</div>
        </div>
        <span class="group-line" />
      </div>

      <div class="setting setting--stack">
        <div class="setting__label">
          <strong>内存分配</strong>
          <span>自动按当前可用内存推荐，随系统占用实时变化；也可以切到自定义手动拖动</span>
        </div>
        <div class="setting__control setting__control--wide">
          <div class="memory">
            <div class="memory__head">
              <div class="memory__modes">
                <button
                  class="memory__mode"
                  :class="{ 'memory__mode--active': form.memory_auto }"
                  @click="setMemoryAuto"
                >
                  自动配置
                </button>
                <button
                  class="memory__mode"
                  :class="{ 'memory__mode--active': !form.memory_auto }"
                  @click="setMemoryManual"
                >
                  自定义
                </button>
              </div>
              <span class="memory__alloc display">{{ memoryGb }} GB</span>
            </div>
            <div class="memory__slider" :class="{ 'memory__slider--off': form.memory_auto }">
              <WoodSlider v-model="form.memory_mb" :min="1024" :max="sliderMax" :step="512" />
            </div>
            <div class="memory__bar">
              <span class="memory__bar-fill" :style="{ width: `${usedPercent}%` }" />
            </div>
            <div class="memory__legend">
              <span>已使用 {{ memoryLabel(usedMb) }} GB / {{ memoryLabel(totalMb) }} GB</span>
              <span>游戏分配 {{ memoryGb }} GB</span>
            </div>
          </div>
        </div>
      </div>

      <div class="setting">
        <div class="setting__label">
          <strong>内存管理</strong>
          <span>决定追加的垃圾回收参数，自定义时不追加</span>
        </div>
        <div class="setting__control">
          <WoodSelect v-model="form.memory_mode" :options="memoryModes" />
        </div>
      </div>

      <div class="setting setting--stack">
        <div class="setting__label">
          <strong>Java 虚拟机参数</strong>
          <span>追加在内存参数之后，空格分隔；可用引号包住带空格的路径</span>
        </div>
        <div class="setting__control setting__control--wide">
          <WoodInput
            v-model="form.java_args"
            mono
            placeholder="-XX:-OmitStackTraceInFastThrow -Dfml.ignorePatchDiscrepancies=true"
          />
        </div>
      </div>

      <div class="setting setting--stack">
        <div class="setting__label">
          <strong>游戏参数</strong>
          <span>追加到游戏主类之后，例如 --width 1600 --height 900</span>
        </div>
        <div class="setting__control setting__control--wide">
          <WoodInput v-model="form.game_args" mono placeholder="--width 1600 --height 900" />
        </div>
      </div>

      <div class="setting setting--stack">
        <div class="setting__label">
          <strong>启动前执行命令</strong>
          <span>工作目录为游戏目录，命中退出码不影响继续启动</span>
        </div>
        <div class="setting__control setting__control--wide">
          <WoodInput v-model="form.pre_launch" mono placeholder="例如 java -jar packwiz-installer-bootstrap.jar ..." />
        </div>
      </div>

      <div class="setting">
        <div class="setting__label">
          <strong>禁用 Java Launch Wrapper</strong>
          <span>不再向游戏注入 DOCL 启动器标识，部分反作弊需要关闭</span>
        </div>
        <div class="setting__control">
          <WoodToggle v-model="form.disable_launch_wrapper" />
        </div>
      </div>

      <div class="setting">
        <div class="setting__label">
          <strong>禁用 LWJGL Unsafe Agent</strong>
          <span>Java 24+ 默认放行 LWJGL 的 Unsafe 内存访问，关闭后可能告警或崩溃</span>
        </div>
        <div class="setting__control">
          <WoodToggle v-model="form.disable_lwjgl_unsafe" />
        </div>
      </div>

      <div class="setting setting--last">
        <div class="setting__label">
          <strong>使用高性能显卡</strong>
          <span>为当前 Java 写入 Windows 显卡偏好，下次启动生效</span>
        </div>
        <div class="setting__control">
          <WoodToggle v-model="form.prefer_gpu" />
        </div>
      </div>
    </section>

    <section
      v-if="settingsTab === 'download'"
      class="card card-pad group reveal"
      :style="{ '--i': 4 }"
    >
      <div class="group-head">
        <div class="group-titles">
          <div class="section-title">下载</div>
        </div>
        <span class="group-line" />
      </div>

      <div class="setting">
        <div class="setting__label">
          <strong>下载源</strong>
          <span>自动模式按来源选择：Modrinth 走镜像，其余优先官方；失败会自动回退</span>
        </div>
        <div class="setting__control">
          <WoodSelect v-model="form.download_source" :options="sourceOptions" />
        </div>
      </div>
      <div class="setting">
        <div class="setting__label">
          <strong>模组中文名</strong>
          <span>从 MC 百科补全中文名，结果缓存 30 天；关闭后只显示源站名称</span>
        </div>
        <div class="setting__control">
          <WoodToggle v-model="form.mcmod_names" />
        </div>
      </div>
      <div class="setting">
        <div class="setting__label">
          <strong>并发下载数</strong>
          <span>当前 {{ form.concurrency }}；网络不稳定时建议降低</span>
        </div>
        <div class="setting__control setting__control--wide">
          <WoodSlider v-model="form.concurrency" :min="1" :max="64" :step="1" />
        </div>
      </div>
      <div class="setting setting--stack">
        <div class="setting__label">
          <strong>下载源测速</strong>
          <span>并发测试官方与镜像的连通性；结果仅供参考，实际下载仍会自动换源</span>
        </div>
        <div class="setting__control setting__control--wide">
          <div class="probe">
            <div class="probe__head">
              <span class="muted small">
                Mod 接口全红时，CurseForge 的模组信息与整合包文件会取不到；游戏文件全红时安装会卡住。
              </span>
              <WoodButton variant="soft" size="sm" :disabled="probeBusy" @click="runProbe">
                <AppIcon name="refresh" :size="14" />
                {{ probeBusy ? "测速中…" : "开始测速" }}
              </WoodButton>
            </div>
            <div v-if="probeGroups.length" class="probe__groups">
              <div v-for="group in probeGroups" :key="group.key" class="probe__group">
                <div class="probe__group-title">{{ group.label }}</div>
                <div v-for="item in group.items" :key="item.id" class="probe__row">
                  <span class="probe__dot" :class="{ 'probe__dot--ok': item.ok }" />
                  <span class="probe__name">{{ item.name }}</span>
                  <span class="probe__ms mono" :class="{ 'probe__ms--err': !item.ok }">
                    {{ item.ok ? `${item.latency_ms} ms` : item.error }}
                  </span>
                </div>
              </div>
            </div>
            <p v-else-if="!probeBusy" class="muted small">还没测过，点「开始测速」会同时请求所有源。</p>
          </div>
        </div>
      </div>

      <div class="setting setting--last">
        <div class="setting__label">
          <strong>速度限制</strong>
          <span>{{ form.speed_limit ? `上限 ${form.speed_limit} MiB/s` : "不限速" }}</span>
        </div>
        <div class="setting__control setting__control--wide">
          <WoodSlider v-model="form.speed_limit" :min="0" :max="64" :step="4" />
        </div>
      </div>
    </section>

    <section
      v-if="settingsTab === 'multiplayer'"
      class="card card-pad group reveal"
      :style="{ '--i': 5 }"
    >
      <div class="group-head">
        <div class="group-titles">
          <div class="section-title">联机</div>
        </div>
        <span class="group-line" />
      </div>

      <div class="setting setting--stack setting--last">
        <div class="setting__label">
          <strong>房间大厅地址</strong>
          <span>留空关闭大厅；填写后可返回 JSON 列表的地址，启动器会按此拉取房间</span>
        </div>
        <div class="setting__control setting__control--wide">
          <WoodInput
            v-model="form.hall_url"
            mono
            placeholder="https://example.com/docl-rooms.json"
          />
        </div>
      </div>

      <p class="muted small settings-hall-hint">
        支持 <span class="mono">{ "rooms": [ ... ] }</span> 或直接返回数组；每个房间至少包含
        <span class="mono">room</span>（陶瓦邀请码），可选 name、motd、players、max_players、version、loader、mods、updated（Unix 秒）。
      </p>

      <div class="setting setting--stack setting--last">
        <div class="setting__label">
          <strong>EasyTier 公共节点</strong>
          <span>
            可选。陶瓦默认使用内置公共节点；直连困难时可填自建或第三方节点，多个用逗号或空格分隔，开房/加入时追加
          </span>
        </div>
        <div class="setting__control setting__control--wide">
          <WoodInput
            v-model="form.terracotta_nodes"
            mono
            placeholder="https://etnode.example.org/node1, https://etnode.example.org/node2"
          />
        </div>
      </div>
    </section>

    <section
      v-if="settingsTab === 'update'"
      class="card card-pad group reveal"
      :style="{ '--i': 6 }"
    >
      <div class="group-head">
        <div class="group-titles">
          <div class="section-title">启动器更新</div>
        </div>
        <span class="group-line" />
        <span v-if="portable" class="tag mono">便携版</span>
        <span class="tag mono">v{{ appVersion || "—" }}</span>
      </div>

      <div class="setting">
        <div class="setting__label">
          <strong>启动器更新</strong>
          <span>启动时按此策略检查；关闭后仍可在这里手动检查</span>
        </div>
        <div class="setting__control">
          <WoodSelect v-model="form.update_mode" :options="updateModes" />
        </div>
      </div>

      <div class="setting setting--stack">
        <div class="setting__label">
          <strong>更新清单地址</strong>
          <span>国内网络选加速地址；需要最快拿到新版选 GitHub 原地址</span>
        </div>
        <div class="setting__control setting__control--wide">
          <div class="update-source">
            <WoodSelect v-model="updateSource" :options="updateSources" />
            <WoodButton
              variant="soft"
              size="sm"
              :disabled="updateBusy || !form.update_url.trim()"
              @click="checkUpdate"
            >
              <AppIcon name="refresh" :size="14" />
              {{ updateBusy ? "检查中…" : "检查更新" }}
            </WoodButton>
          </div>
          <WoodInput
            v-model="form.update_url"
            mono
            :disabled="updateSource !== 'custom'"
            placeholder="https://example.com/docl-update.json"
          />
        </div>
      </div>

      <p class="muted small update-hint">
        清单格式：
        <span class="mono">{ "version": "0.2.0", "url": "https://…/DOCL_0.2.0-setup.exe", "portable_url": "https://…/DOCL_0.2.0_portable.zip", "notes": "更新说明" }</span>。
        <template v-if="portable">
          便携版下载后点「更新并重启」，自动完成替换，全程不弹安装向导。
        </template>
        <template v-else>
          发现新版本后下载的是完整安装包，点击安装时启动器会自动退出。
        </template>
      </p>

      <div v-if="updateError" class="update-error">
        <AppIcon name="alert" :size="14" />
        {{ updateError }}
        <WoodButton variant="ghost" size="sm" @click="checkUpdate">重试</WoodButton>
      </div>

      <div v-else-if="updateInfo" class="update-panel">
        <div class="update-panel__head">
          <strong>
            {{ updateInfo.available ? `发现新版本 ${updateInfo.latest}` : "已经是最新版本" }}
          </strong>
          <span class="muted small mono">当前 v{{ updateInfo.current }}</span>
        </div>
        <p v-if="updateInfo.notes" class="update-notes">{{ updateInfo.notes }}</p>
        <div v-if="updateInfo.available" class="update-actions">
          <div v-if="updateProgress" class="update-progress">
            <span class="update-progress__bar">
              <span :style="{ width: `${updatePercent}%` }" />
            </span>
            <span class="muted small mono">{{ updateProgressText }}</span>
          </div>
          <WoodButton
            v-if="!updatePath"
            variant="primary"
            size="sm"
            :disabled="updateBusy"
            @click="downloadUpdate"
          >
            <AppIcon name="download" :size="14" />
            {{ updateBusy ? "下载中…" : isPortablePack ? "下载便携包" : "下载安装包" }}
          </WoodButton>
          <WoodButton
            v-else-if="isPortablePack"
            variant="primary"
            size="sm"
            :disabled="applyingUpdate"
            @click="applyPortableUpdate"
          >
            <AppIcon name="refresh" :size="14" />
            {{ applyingUpdate ? "正在更新…" : "更新并重启" }}
          </WoodButton>
          <WoodButton v-else variant="primary" size="sm" @click="installUpdate">
            <AppIcon name="check" :size="14" />
            安装并退出
          </WoodButton>
        </div>
      </div>
    </section>
      </div>
    </div>
  </div>
</template>

<style scoped>
.settings-hall-hint {
  margin: 0;
  line-height: 1.6;
}

.update-source {
  display: flex;
  align-items: center;
  gap: 10px;
}

.update-source :deep(.select) {
  flex: 1;
  min-width: 0;
}

.update-hint {
  margin: 0;
  line-height: 1.7;
}

.update-error {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 10px 12px;
  border-radius: 4px;
  border: 1px solid var(--line-strong);
  background: var(--danger-soft);
  color: var(--danger);
  font-size: 12px;
}

.update-error .btn {
  margin-left: auto;
}

.update-panel {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 12px 14px;
  border: 1px solid var(--line-strong);
  border-radius: 6px;
  background: var(--panel-2);
}

.update-panel__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.update-panel__head strong {
  font-size: 13px;
  color: var(--ink);
}

.update-notes {
  margin: 0;
  font-size: 12px;
  color: var(--ink-soft);
  line-height: 1.6;
  white-space: pre-wrap;
}

.update-actions {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}

.update-progress {
  flex: 1;
  min-width: 180px;
  display: flex;
  align-items: center;
  gap: 8px;
}

.update-progress__bar {
  flex: 1;
  height: 6px;
  border-radius: 3px;
  background: rgba(var(--accent-rgb), 0.12);
  overflow: hidden;
}

.update-progress__bar > span {
  display: block;
  height: 100%;
  border-radius: 3px;
  background: linear-gradient(90deg, var(--moss), var(--walnut-light));
  transition: width 0.2s ease;
}

.settings-layout {
  display: grid;
  grid-template-columns: 190px minmax(0, 1fr);
  gap: 16px;
  align-items: start;
}

.settings-side {
  padding: 10px 8px;
  display: flex;
  flex-direction: column;
  gap: 2px;
  position: sticky;
  top: 0;
}

.settings-side__item {
  position: relative;
  display: flex;
  align-items: center;
  gap: 9px;
  height: 40px;
  padding: 0 10px;
  border-radius: 4px;
  color: var(--ink-soft);
  font-size: 12.5px;
  font-weight: 600;
  text-align: left;
  transition: background var(--dur-2) ease, color var(--dur-2) ease;
}

.settings-side__item:hover {
  background: rgba(var(--accent-rgb), 0.06);
  color: var(--walnut-deep);
}

.settings-side__item--active {
  background: rgba(var(--accent-rgb), 0.07);
  color: var(--walnut-deep);
}

.settings-side__item--active::before {
  content: "";
  position: absolute;
  left: 0;
  top: 8px;
  bottom: 8px;
  width: 2px;
  background: var(--moss);
}

.settings-side__item--active .icon {
  color: var(--moss);
}

.settings-side__label {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.settings-main {
  display: flex;
  flex-direction: column;
  gap: 16px;
  min-width: 0;
}

.group-head {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 6px;
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
  margin: 0 4px;
}

.setting {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 24px;
  padding: 14px 0;
  border-bottom: 1px dashed var(--line);
}

.setting--last {
  border-bottom: none;
  padding-bottom: 2px;
}

/* 目录这类内容较长的设置改为上下布局，避免左右互相挤压 */
.setting--stack {
  flex-direction: column;
  align-items: stretch;
  gap: 10px;
}

.setting--stack .setting__control {
  flex-direction: column;
  align-items: stretch;
  gap: 8px;
  width: 100%;
  max-width: none;
}

.setting--stack .row {
  width: 100%;
}

.setting--stack .row .input {
  flex: 1;
  min-width: 0;
}

.setting__label {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
}

.setting__label strong {
  font-size: 13.5px;
  color: var(--ink);
}

.setting__label span {
  font-size: 12px;
  color: var(--muted);
}

.setting__control {
  flex-shrink: 0;
  min-width: 150px;
  display: flex;
  justify-content: flex-end;
}

/* 下拉框按固定宽度显示，长选项在内部省略而不是把整行撑破 */
.setting__control .select {
  width: 230px;
  min-width: 0;
  max-width: 100%;
}

.setting__control .row .select {
  width: 180px;
}

.setting__control--wide {
  flex: 1;
  max-width: 430px;
}

.resolution {
  width: 300px;
}

.memory {
  width: 100%;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.memory__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.memory__modes {
  display: flex;
  gap: 3px;
  padding: 3px;
  border-radius: 4px;
  border: 1px solid var(--line);
  background: var(--panel-2);
}

.memory__mode {
  height: 28px;
  padding: 0 13px;
  border-radius: 3px;
  color: var(--ink-soft);
  font-size: 12px;
  font-weight: 600;
  transition: background var(--dur-1) ease, color var(--dur-1) ease,
    box-shadow var(--dur-1) ease;
}

.memory__mode:hover {
  color: var(--walnut-deep);
}

.memory__mode--active {
  background: var(--panel);
  color: var(--walnut-deep);
  box-shadow: var(--shadow-1);
}

.memory__alloc {
  font-size: 24px;
  line-height: 1;
  color: var(--walnut-deep);
}

.memory__slider--off {
  opacity: 0.55;
  pointer-events: none;
}

.memory__bar {
  height: 8px;
  border-radius: 3px;
  border: 1px solid var(--line);
  background: var(--paper-2);
  overflow: hidden;
}

.memory__bar-fill {
  display: block;
  height: 100%;
  border-radius: 3px;
  background: linear-gradient(90deg, var(--moss), var(--moss-bright));
  transition: width var(--dur-3) var(--ease-out-expo);
}

.memory__legend {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  font-size: 11.5px;
  color: var(--muted);
}

.resolution__x {
  font-size: 13px;
}

.dir-current {
  display: flex;
  flex-direction: column;
  gap: 8px;
  width: 100%;
  padding: 12px 14px;
  border: 1px solid var(--line);
  border-radius: 6px;
  background: var(--panel-2);
}

.dir-current__path {
  overflow: hidden;
  color: var(--muted);
  font-size: 12px;
  text-overflow: ellipsis;
  white-space: nowrap;
  user-select: text;
}

.dir-current__actions {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  flex-wrap: wrap;
}

.dir-link {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 0;
  color: var(--moss);
  font-size: 11.5px;
  font-weight: 700;
}

.dir-link:hover {
  text-decoration: underline;
}

.dir-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
  width: 100%;
  padding: 6px;
  border: 1px solid var(--line);
  border-radius: 6px;
  background: var(--panel);
}

.dir-row {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 0 2px;
  border: 1px solid transparent;
  border-radius: 4px;
  background: transparent;
  transition: background var(--dur-2) ease, border-color var(--dur-2) ease;
}

.dir-row:hover {
  background: var(--panel-2);
}

.dir-row--active {
  border-color: color-mix(in srgb, var(--moss) 42%, transparent);
  background: var(--moss-soft);
}

.dir-row__count {
  flex-shrink: 0;
  color: var(--muted);
  font-size: 10px;
}

.dir-row__main {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 9px;
  height: 42px;
  padding: 0 9px;
  border-radius: 3px;
  text-align: left;
  color: var(--ink-soft);
}

.dir-row__text {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.dir-row__text strong {
  font-size: 12.5px;
  color: var(--walnut-deep);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.dir-row__text span {
  font-size: 10.5px;
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.dir-actions {
  margin-top: 8px;
}

.path-card {
  display: flex;
  flex-direction: column;
  gap: 8px;
  width: 100%;
  padding: 10px 12px;
  border: 1px solid var(--line);
  border-radius: 6px;
  background: var(--panel-2);
}

.path-card__text {
  overflow: hidden;
  color: var(--muted);
  font-size: 11.5px;
  text-overflow: ellipsis;
  white-space: nowrap;
  user-select: text;
}

.path-card__foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}

.path-card__buttons {
  display: flex;
  align-items: center;
  gap: 8px;
}

/* 内置底图改成一排缩略图卡片 */
.bg-tiles {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(104px, 1fr));
  gap: 10px;
  width: 100%;
}

.bg-tile {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 6px;
  border-radius: 6px;
  border: 1px solid var(--line);
  background: var(--panel-2);
  transition: border-color var(--dur-2) ease, box-shadow var(--dur-2) ease,
    transform var(--dur-2) var(--ease-out-expo);
}

.bg-tile:hover {
  border-color: var(--walnut-light);
  transform: translateY(-1px);
}

.bg-tile--active {
  border-color: var(--moss);
  box-shadow: 0 0 0 3px rgba(var(--accent-rgb), 0.08);
}

.bg-tile__thumb {
  display: block;
  width: 100%;
  height: 56px;
  border-radius: 4px;
  border: 1px solid rgba(0, 0, 0, 0.08);
  background-size: cover;
  background-position: center;
}

.bg-tile__label {
  padding-left: 2px;
  color: var(--ink-soft);
  font-size: 11.5px;
  font-weight: 700;
  text-align: left;
}

.java-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
  margin: 14px 0;
  padding: 6px;
  border: 1px solid var(--line);
  border-radius: 6px;
  background: var(--panel);
}

.java-item {
  display: flex;
  align-items: center;
  gap: 13px;
  padding: 10px 12px;
  border-radius: 4px;
  border: 1px solid transparent;
  background: transparent;
  text-align: left;
  position: relative;
  overflow: hidden;
  cursor: pointer;
  transition: background var(--dur-2) ease, border-color var(--dur-2) ease;
}

.java-item:hover {
  background: var(--panel-2);
}

.java-item--active {
  border-color: color-mix(in srgb, var(--moss) 42%, transparent);
  background: var(--moss-soft);
}

.java-item--active::after {
  content: "";
  position: absolute;
  left: 0;
  top: 0;
  bottom: 0;
  width: 3px;
  background: var(--moss);
}

.java-item__major {
  font-size: 18px;
  color: var(--walnut-deep);
  min-width: 84px;
}

.java-item__path {
  flex: 1;
  min-width: 0;
  font-size: 11.5px;
  color: var(--ink-soft);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.java-item__check {
  color: var(--moss);
}

.java-empty {
  padding: 18px;
  border-radius: 4px;
  background: var(--panel-2);
  border: 1px dashed var(--line-strong);
  color: var(--muted);
  font-size: 12.5px;
  text-align: center;
}

.java-actions {
  justify-content: space-between;
}

/* 运行时发行版/版本下拉固定宽度，避免和「下载并安装」按钮一起撑出卡片 */
.runtime-source {
  width: 160px;
}

.runtime-select {
  width: 110px;
}

.java-progress {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 10px;
}

.runtime-action {
  width: 118px;
}

.theme-cards {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(104px, 1fr));
  gap: 10px;
  width: 100%;
}

.theme-card {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 6px;
  border-radius: 6px;
  border: 1px solid var(--line);
  background: var(--panel-2);
  transition: border-color var(--dur-2) ease, box-shadow var(--dur-2) ease,
    transform var(--dur-2) var(--ease-out-expo);
}

.theme-card:hover {
  border-color: var(--walnut-light);
  transform: translateY(-1px);
}

.theme-card--active {
  border-color: var(--moss);
  box-shadow: 0 0 0 3px rgba(var(--accent-rgb), 0.08);
}

.theme-card__preview {
  position: relative;
  height: 54px;
  border-radius: 4px;
  overflow: hidden;
  background: var(--pv-bg);
  border: 1px solid rgba(0, 0, 0, 0.08);
}

.theme-card__side {
  position: absolute;
  left: 0;
  top: 0;
  bottom: 0;
  width: 22%;
  background: color-mix(in srgb, var(--pv-main) 24%, transparent);
  border-right: 1px solid rgba(0, 0, 0, 0.06);
}

.theme-card__bar {
  position: absolute;
  left: 30%;
  top: 11px;
  width: 28%;
  height: 5px;
  border-radius: 2px;
  background: var(--pv-main);
}

.theme-card__line {
  position: absolute;
  left: 30%;
  right: 12%;
  top: 35px;
  height: 3px;
  border-radius: 2px;
  background: color-mix(in srgb, var(--pv-main) 30%, transparent);
}

.theme-card__line--long {
  top: 24px;
  right: 24%;
}

.theme-card__name {
  padding-left: 2px;
  color: var(--ink-soft);
  font-size: 11.5px;
  font-weight: 700;
  text-align: left;
}

/* 外观选项预览卡（窗口材质 / 密度 / 圆角） */
.option-cards {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
  width: 100%;
}

.option-card {
  display: flex;
  flex-direction: column;
  gap: 6px;
  width: 112px;
  padding: 6px;
  border-radius: 6px;
  border: 1px solid var(--line);
  background: var(--panel-2);
  transition: border-color var(--dur-2) ease, box-shadow var(--dur-2) ease,
    transform var(--dur-2) var(--ease-out-expo);
}

.option-card:hover {
  border-color: var(--walnut-light);
  transform: translateY(-1px);
}

.option-card--active {
  border-color: var(--moss);
  background: var(--moss-soft);
  box-shadow: 0 0 0 3px rgba(var(--accent-rgb), 0.08);
}

.option-card__name {
  color: var(--ink-soft);
  font-size: 11.5px;
  font-weight: 700;
  text-align: left;
}

.option-card__preview {
  position: relative;
  display: block;
  height: 46px;
  border-radius: 4px;
  border: 1px solid rgba(0, 0, 0, 0.08);
  overflow: hidden;
}

.option-card__preview--font {
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--panel);
  color: var(--walnut-deep);
  font-size: 17px;
  letter-spacing: 0.02em;
}

.option-card__preview--solid {
  background: linear-gradient(160deg, var(--panel-2), var(--panel));
}

.option-card__preview--solid::after {
  content: "";
  position: absolute;
  inset: 8px 10px auto 10px;
  height: 24px;
  border-radius: 3px;
  background: var(--panel);
  border: 1px solid var(--line);
}

.option-card__preview--glass {
  background: radial-gradient(circle at 28% 30%, rgba(255, 255, 255, 0.6), transparent 62%),
    linear-gradient(160deg, #90b2a2, #cbd6c5);
}

.option-card__preview--glass::after {
  content: "";
  position: absolute;
  inset: 8px 10px auto 10px;
  height: 24px;
  border-radius: 3px;
  background: rgba(255, 255, 255, 0.5);
  border: 1px solid rgba(255, 255, 255, 0.55);
  backdrop-filter: blur(4px);
}

.option-card__preview--density {
  display: flex;
  flex-direction: column;
  justify-content: center;
  padding: 0 12px;
  background: var(--panel);
}

.option-card__preview--density i {
  display: block;
  height: 3px;
  border-radius: 2px;
  background: var(--line-strong);
}

.option-card__preview--density i:last-child {
  width: 58%;
}

.density--compact i + i {
  margin-top: 3px;
}

.density--normal i + i {
  margin-top: 6px;
}

.density--comfy i + i {
  margin-top: 9px;
}

.option-card__preview--radius {
  background: var(--panel);
}

.option-card__preview--radius::after {
  content: "";
  position: absolute;
  inset: 8px 16px;
  background: var(--panel-2);
  border: 1px solid var(--line);
}

.radius--sharp::after {
  border-radius: 2px;
}

.radius--default::after {
  border-radius: 7px;
}

.radius--round::after {
  border-radius: 14px;
}

/* 背景预览 */
.bg-preview {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 100%;
  max-width: 560px;
  height: 156px;
  padding: 10px;
  border-radius: 8px;
  border: 1px solid var(--line);
  background-color: var(--panel-2);
  background-position: center;
  background-repeat: no-repeat;
  overflow: hidden;
}

.bg-preview--empty {
  background-image: repeating-linear-gradient(
    135deg,
    transparent 0 8px,
    rgba(var(--accent-rgb), 0.06) 8px 16px
  );
}

.bg-preview__empty {
  padding: 0 14px;
  color: var(--muted);
  font-size: 11.5px;
  text-align: center;
}

.bg-preview__pick {
  position: absolute;
  right: 10px;
  bottom: 10px;
  height: 26px;
  padding: 0 12px;
  border-radius: 99px;
  border: 1px solid rgba(255, 255, 255, 0.6);
  background: rgba(255, 255, 255, 0.82);
  color: var(--walnut-deep);
  font-size: 11.5px;
  font-weight: 700;
  box-shadow: 0 2px 6px rgba(60, 44, 26, 0.16);
}

.bg-preview__pick:hover {
  background: #fff;
}

.bg-preview__actions {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  width: 100%;
  margin-top: 8px;
  flex-wrap: wrap;
}

.bg-presets {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  width: 100%;
}

.bg-chip {
  display: flex;
  align-items: center;
  gap: 7px;
  height: 34px;
  padding: 0 10px;
  border-radius: 4px;
  border: 1px solid var(--line);
  background: var(--panel-2);
  color: var(--ink-soft);
  font-size: 12px;
  font-weight: 600;
  transition: border-color var(--dur-2) ease, transform var(--dur-2) var(--ease-out-expo);
}

.bg-chip:hover {
  border-color: var(--walnut-light);
  transform: translateY(-1px);
}

.bg-chip--active {
  border-color: var(--moss);
  color: var(--moss);
  box-shadow: 0 0 0 3px rgba(var(--accent-rgb), 0.08);
}

.bg-chip__swatch {
  width: 22px;
  height: 16px;
  border-radius: 4px;
  border: 1px solid rgba(0, 0, 0, 0.14);
  background-size: cover;
}

.bg-chip__swatch--forest {
  background-image: linear-gradient(180deg, #b7ccb0 0 40%, #7d9a6b 41% 60%, #45603c 61%);
}

.bg-chip__swatch--dune {
  background-image: linear-gradient(180deg, #f7dfc0 0 40%, #d9a97f 41% 60%, #a8623f 61%);
}

.bg-chip__swatch--night {
  background-image: linear-gradient(180deg, #141d2e 0 46%, #1e2c42 47% 72%, #16202e 73%);
}

.bg-chip__swatch--sakura {
  background-image: linear-gradient(180deg, #fdeef2 0 38%, #f8d7e1 39% 66%, #e6a3ba 67%);
}

.bg-chip__swatch--ocean {
  background-image: linear-gradient(180deg, #dff1f7 0 36%, #5f9cb2 37% 66%, #2f6076 67%);
}

.bg-chip__swatch--aurora {
  background-image: radial-gradient(circle at 30% 25%, #78eabe 0 18%, transparent 40%),
    radial-gradient(circle at 70% 45%, #9682f0 0 14%, transparent 36%),
    linear-gradient(180deg, #101a2c 0 46%, #172a42 47% 72%, #101a2a 73%);
}

.bg-chip__swatch--parchment {
  background-image: radial-gradient(ellipse at 20% 16%, rgba(120, 94, 62, 0.3), transparent 60%),
    linear-gradient(180deg, #f3ebd9 0 40%, #e9dcc3 41% 62%, #dbc8a8 63%);
}

.bg-chip__swatch--porcelain {
  background-image: radial-gradient(circle at 26% 22%, rgba(63, 107, 135, 0.35), transparent 52%),
    radial-gradient(circle at 76% 68%, rgba(63, 107, 135, 0.26), transparent 50%),
    linear-gradient(180deg, #f6faf8 0 40%, #e6f0ec 41% 66%, #d2e2dc 67%);
}

.bg-chip__swatch--cloud {
  background-image: radial-gradient(circle at 76% 18%, #fffcf0 0 8%, transparent 18%),
    radial-gradient(ellipse at 24% 70%, #ffffff 0 20%, transparent 50%),
    linear-gradient(180deg, #eaf1f7 0 44%, #d7e4ee 45% 68%, #bfd1df 69%);
}

.bg-chip__swatch--mist {
  background-image: radial-gradient(ellipse at 30% 28%, rgba(180, 156, 203, 0.6), transparent 60%),
    radial-gradient(ellipse at 74% 74%, rgba(150, 120, 190, 0.55), transparent 60%),
    linear-gradient(180deg, #f5f0fa 0 42%, #e7def3 43% 68%, #d5c7e6 69%);
}

.bg-chip__swatch--grid {
  background-image: linear-gradient(180deg, #14171a 0 46%, #1b1f24 47% 72%, #121518 73%);
}

.bg-chip__swatch--neon {
  background-image: radial-gradient(circle at 18% 24%, rgba(34, 211, 238, 0.65) 0 16%, transparent 38%),
    radial-gradient(circle at 82% 36%, rgba(244, 114, 182, 0.6) 0 14%, transparent 34%),
    linear-gradient(180deg, #0a0814 0 46%, #120f26 47% 72%, #0a0816 73%);
}

.bg-chip__swatch--wheat {
  background-image: radial-gradient(circle at 26% 18%, #fff4c8 0 9%, transparent 20%),
    radial-gradient(ellipse at 20% 108%, #b98f34 0 58%, transparent 59%),
    linear-gradient(180deg, #f8ecc4 0 46%, #ecd89b 47% 72%, #cda851 73%);
}

.bg-chip__swatch--snow {
  background-image: radial-gradient(circle at 74% 16%, #ffffff 0 8%, transparent 18%),
    radial-gradient(ellipse at 14% 108%, #e8f1f8 0 58%, transparent 59%),
    linear-gradient(180deg, #dfeefa 0 46%, #cbe1f1 47% 72%, #accbe4 73%);
}

.bg-chip__swatch--none {
  background-image: linear-gradient(135deg, var(--panel) 0 50%, var(--panel-2) 50%);
}

/* ---------- 下载源测速 ---------- */
.probe {
  display: flex;
  flex-direction: column;
  gap: 10px;
  width: 100%;
}

.probe__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}

.probe__groups {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.probe__group {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.probe__group-title {
  font-size: 12px;
  font-weight: 800;
  color: var(--walnut-deep);
}

.probe__row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 9px;
  border: 1px solid var(--line);
  border-radius: 4px;
  background: var(--panel-2);
  font-size: 12px;
}

.probe__dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--danger);
  flex-shrink: 0;
}

.probe__dot--ok {
  background: var(--moss);
}

.probe__name {
  flex: 1;
  min-width: 0;
  color: var(--ink);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.probe__ms {
  flex-shrink: 0;
  font-size: 11.5px;
  color: var(--ink-soft);
}

.probe__ms--err {
  color: var(--danger);
  max-width: 55%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 版本页卡片底图选择 */
.profile-bg-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px;
  width: 100%;
}

.profile-bg-card {
  display: grid;
  grid-template-columns: 96px minmax(0, 1fr);
  grid-template-rows: auto auto;
  gap: 8px 12px;
  padding: 10px;
  border: 1px solid var(--line);
  border-radius: 8px;
  background: rgba(var(--glass-rgb), 0.82);
}

.color-well {
  width: 34px;
  height: 30px;
  padding: 0;
  border: 1px solid var(--line);
  border-radius: 4px;
  background: transparent;
  cursor: pointer;
}

.color-well::-webkit-color-swatch-wrapper {
  padding: 3px;
}

.color-well::-webkit-color-swatch {
  border: none;
  border-radius: 2px;
}

.profile-bg-card__preview {
  grid-row: 1 / span 2;
  width: 96px;
  height: 76px;
  border: 1px solid var(--line);
  border-radius: 7px;
  background-color: var(--panel-2);
  background-position: center;
  background-size: cover;
}

.profile-bg-card__body {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.profile-bg-card__body strong {
  font-size: 13.5px;
  color: var(--walnut-deep);
}

.profile-bg-card__body span {
  overflow: hidden;
  color: var(--muted);
  font-size: 11px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.profile-bg-card__actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
}

@media (max-width: 1040px) {
  .profile-bg-grid {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
