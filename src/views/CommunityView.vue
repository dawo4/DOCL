<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from "vue";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  api,
  openExternal,
  type CfFileView,
  type CfProject,
  type InstanceContext,
  type PackPreview,
} from "../api";
import AppIcon from "../components/AppIcon.vue";
import IconButton from "../components/IconButton.vue";
import WoodButton from "../components/WoodButton.vue";
import WoodInput from "../components/WoodInput.vue";
import WoodSelect from "../components/WoodSelect.vue";
import WoodToggle from "../components/WoodToggle.vue";
import {
  downloadModpack,
  importModpack,
  installCfFile,
  installShaderLoader,
  installWorld,
  notify,
  openInstance,
  refreshVersions,
  store,
} from "../store";
import { formatBytes, formatDate } from "../utils";

const props = defineProps<{ kind: string }>();

const preset = store.communityContext;

type SourceKey = "curseforge" | "modrinth";

interface Bucket {
  items: CfProject[];
  total: number;
  next: number;
  done: boolean;
}

const CLASS_LABELS: Record<string, string> = {
  mods: "Mod",
  resourcepacks: "资源包",
  shaders: "光影包",
  modpacks: "整合包",
  datapacks: "数据包",
  worlds: "地图",
};

const CLASS_IDS: Record<string, number> = {
  mods: 6,
  resourcepacks: 12,
  shaders: 6552,
  datapacks: 6945,
  modpacks: 4471,
  worlds: 17,
};

const LOADER_LABELS: Record<string, string> = {
  vanilla: "原版",
  fabric: "Fabric",
  quilt: "Quilt",
  forge: "Forge",
  neoforge: "NeoForge",
};

/// CurseForge / Modrinth 分类英文名到中文的映射，未命中的显示原文。
const CATEGORY_ZH: Record<string, string> = {
  // 两个来源通用的整合包分类
  adventure: "冒险",
  "adventure and rpg": "冒险与 RPG",
  technology: "科技",
  tech: "科技",
  magic: "魔法",
  quests: "任务",
  quest: "任务",
  hardcore: "硬核",
  multiplayer: "多人",
  "sci-fi": "科幻",
  skyblock: "空岛",
  exploration: "探索",
  combat: "战斗",
  horror: "恐怖",
  cursed: "诅咒",
  food: "食物",
  "vanilla+": "原版增强",
  vanilla: "原版",
  ftb: "FTB 官方",
  "ftb official pack": "FTB 官方",
  "small / light": "小游戏/轻量",
  small: "小型",
  light: "轻量",
  medium: "中型",
  large: "大型",
  "extra large": "超大型",
  "extra-large": "超大型",
  "map based": "地图",
  budget: "低配",

  // Modrinth 通用功能分类
  utility: "实用",
  optimization: "性能优化",
  equipment: "装备",
  "game-mechanics": "游戏机制",
  "game mechanic": "游戏机制",
  logistics: "物流",
  economy: "经济",
  decoration: "装饰",
  education: "教育",
  social: "社交",
  storage: "存储",
  transportation: "运输",
  library: "支持库",
  worldgen: "世界生成",
  "world gen": "世界生成",
  mobs: "生物",
  "quality of life": "实用与优化",

  // CurseForge Mod 分类
  addons: "附属",
  automation: "自动化",
  "blood magic": "血魔法",
  buildcraft: "建筑",
  cosmetic: "装饰",
  energy: "能源",
  farming: "农业",
  inventory: "物品栏",
  "map and information": "地图与信息",
  performance: "性能优化",
  processing: "加工",
  teleportation: "传送",
  thaumcraft: "神秘时代",
  "tinkers construct": "匠魂",
  tutorials: "教程",
  "utility & qol": "实用与优化",

  // 资源包 / 光影风格（分辨率保留原样）
  traditional: "传统风",
  simplistic: "简约风",
  realistic: "写实",
  "photo realistic": "照片级写实",
  "semi-realistic": "半写实",
  "semi realistic": "半写实",
  modern: "现代风",
  themed: "主题",
  medieval: "中世纪",
  steampunk: "蒸汽朋克",
  animated: "动画",
  "mod support": "模组支持",
  "font packs": "字体",
  gui: "界面",
  misc: "杂项",
  "misc.": "杂项",
  fantasy: "奇幻",
  cartoony: "卡通风",
  cartoon: "卡通风",
  "vanilla-like": "原版风",
  vibrant: "鲜艳",

  // 以下按 CurseForge / Modrinth 真实分类全量补译
  "admin tools": "管理工具",
  "anti-griefing tools": "防破坏工具",
  "applied energistics 2": "应用能源 2",
  "armor, tools, and weapons": "护甲与武器",
  biomes: "生物群系",
  "bug fixes": "错误修复",
  "building gadgets": "建筑装置",
  "bukkit plugins": "Bukkit 插件",
  challenging: "高难度",
  "chat related": "聊天相关",
  "combat / pvp": "战斗 / PvP",
  configuration: "配置",
  create: "机械动力",
  creation: "创作",
  creativemode: "创造模式",
  customization: "自定义",
  "data packs": "数据包",
  "developer tools": "开发者工具",
  dimensions: "维度",
  "energy, fluid, and item transport": "能源、流体与物品运输",
  expert: "专家级",
  "farmer's delight": "农夫乐事",
  fixes: "修复",
  forestry: "林业",
  fun: "娱乐",
  galacticraft: "星际探险",
  "game map": "游戏地图",
  general: "通用",
  genetics: "基因",
  guidebook: "指南书",
  "hardcore questing mode": "硬核任务模式",
  "industrial craft": "工业",
  informational: "信息展示",
  "integrated dynamics": "集成动力",
  "kitchen-sink": "大杂烩",
  lightweight: "轻量",
  "lucky blocks": "幸运方块",
  management: "管理",
  mechanics: "机制",
  "mini game": "小游戏",
  minigame: "小游戏",
  modded: "模组化",
  "modded world": "模组世界",
  modpacks: "整合包",
  mods: "模组",
  "ores and resources": "矿物与资源",
  parkour: "跑酷",
  "player transport": "玩家运输",
  progression: "进阶",
  puzzle: "解谜",
  recipes: "配方",
  redstone: "红石",
  "refined storage": "精妙存储",
  "resource packs": "资源包",
  "role playing": "角色扮演",
  scenarios: "剧情",
  scripts: "脚本",
  "server utility": "服务器实用",
  shaders: "光影",
  structures: "建筑结构",
  survival: "生存",
  "tinker's construct": "匠魂",
  tweaks: "微调",
  "thermal expansion": "热力膨胀",
  "twilight forest": "暮色森林",
  "twitch integration": "Twitch 集成",
  "website administration": "网站管理",
  "world editing and management": "世界编辑与管理",
  "world generators": "世界生成器",
  worlds: "世界",
  "api and library": "API 与支持库",
  miscellaneous: "杂项",
  "512x and higher": "512x 及以上",
};

function zhCategory(name: string) {
  return CATEGORY_ZH[name.toLowerCase()] ?? name;
}

/// 地图只有 CurseForge 收录，Modrinth 没有对应类型
const sourceOptions = computed(() =>
  props.kind === "worlds"
    ? [{ value: "curseforge", label: "CurseForge" }]
    : [
        { value: "all", label: "全部来源" },
        { value: "curseforge", label: "CurseForge" },
        { value: "modrinth", label: "Modrinth" },
      ]
);

function emptyBuckets(): Record<SourceKey, Bucket> {
  return {
    curseforge: { items: [], total: 0, next: 0, done: false },
    modrinth: { items: [], total: 0, next: 0, done: false },
  };
}

const instanceId = ref<string>("");
const context = ref<InstanceContext | null>(null);
const query = ref("");
const source = ref<"all" | SourceKey>("all");
const versionMode = ref("all");
const sortField = ref("2");
const category = ref("");
const versions = ref<string[]>([]);
const cfCategories = ref<{ id: number; name: string }[]>([]);
const mrCategories = ref<string[]>([]);
const buckets = ref<Record<SourceKey, Bucket>>(emptyBuckets());
const loading = ref(false);
const error = ref("");
const active = ref<CfProject | null>(null);
const projectStack = ref<CfProject[]>([]);
const files = ref<CfFileView[]>([]);
const depFile = ref<CfFileView | null>(null);
const depProjects = ref<CfProject[]>([]);
const depLoading = ref(false);
const depError = ref("");
const filesLoading = ref(false);
const filesError = ref("");
/// 文件列表按 MC 版本筛选的 tab
const versionTab = ref("all");
/// 光影类目下自动检测出的加载器推荐（缺加载器时展示）
const shaderLoaderHint = ref<{
  name: string;
  summary: string;
  /// 推荐的光影加载器项目；原版实例没有
  project: CfProject | null;
  /// 原版实例需要先装的加载器
  vanilla: { loader: string; gameVersion: string } | null;
} | null>(null);

const showImport = ref(false);
const packPath = ref("");
const packPreview = ref<PackPreview | null>(null);
const inspecting = ref(false);
const importForm = reactive({
  name: "",
  id: "",
  loaderVersion: "",
  parentVersion: "",
  optional: false,
  /// 安装为服务端：按清单跳过仅客户端文件
  server: false,
});

const label = computed(() => CLASS_LABELS[props.kind] ?? "资源");

/// 当前资源的外部页面链接（来源站点项目页 + MC 百科搜索）
const projectLinks = computed(() => {
  const project = active.value;
  if (!project) return [];
  const links: { label: string; url: string }[] = [];
  if (project.website_url) {
    links.push({
      label: project.source === "modrinth" ? "转到 Modrinth" : "转到 CurseForge",
      url: project.website_url,
    });
  }
  links.push({
    label: "转到 MC 百科",
    url:
      mcNames.value[`${project.source}-${project.id}`]?.url ||
      `https://search.mcmod.cn/s?key=${encodeURIComponent(project.name)}&site=mcm&filter=0`,
  });
  return links;
});

async function copyProjectName() {
  if (!active.value) return;
  try {
    await navigator.clipboard.writeText(active.value.name);
    notify("名称已复制");
  } catch {
    notify("复制失败，请手动选中复制", "err");
  }
}

/// 文件列表的版本 tab：与官方版本表求交集，滤掉 CurseForge 混入的
/// 加载器 / 环境 token（Fabric、Client、Java 17 等），按版本发布时间新→旧取前 11 个
const versionTabs = computed(() => {
  const official = new Map(store.versions.map((v) => [v.id, v.release_time]));
  const counter = new Map<string, number>();
  for (const file of files.value) {
    for (const version of file.game_versions) {
      if (!official.has(version)) continue;
      counter.set(version, (counter.get(version) ?? 0) + 1);
    }
  }
  const top = [...counter.keys()]
    .sort((a, b) => (official.get(b) ?? "").localeCompare(official.get(a) ?? ""))
    .slice(0, 11);
  return ["all", ...top];
});

const visibleFiles = computed(() =>
  versionTab.value === "all"
    ? files.value
    : files.value.filter((file) => file.game_versions.includes(versionTab.value))
);

watch([active, files], () => {
  versionTab.value = "all";
});

/// 进入光影包详情或切换实例时，自动检测并展示缺失的光影加载器
watch([active, instanceId, () => props.kind], () => {
  void loadShaderLoaderHint();
});
const parentOptions = computed(() =>
  store.installed
    .filter((item) => !item.dependency)
    .map((item) => ({ value: item.id, label: item.id }))
);
const instanceOptions = computed(() => [
  // 允许只浏览搜索结果，不绑定任何实例
  { value: "", label: "不指定（仅浏览）" },
  ...store.installed
    .filter((item) => !item.dependency)
    .map((v) => ({ value: v.id, label: v.id })),
]);
const versionOptions = computed(() => [
  {
    value: "instance",
    label: context.value ? `跟随实例 · ${context.value.game_version}` : "跟随实例",
  },
  { value: "all", label: "全部版本" },
  ...versions.value.map((version) => ({ value: version, label: version })),
]);
const sortOptions = [
  { value: "2", label: "按热度" },
  { value: "3", label: "最近更新" },
  { value: "6", label: "总下载量" },
];
const typeOptions = computed(() => {
  if (source.value === "curseforge") {
    return cfCategories.value.map((item) => ({
      value: String(item.id),
      label: zhCategory(item.name),
    }));
  }
  if (source.value === "modrinth") {
    return mrCategories.value.map((name) => ({
      value: name,
      label: zhCategory(name),
    }));
  }
  return [];
});
/// 两个来源的分类体系不同，聚合浏览时不提供类型筛选
const typeDisabled = computed(
  () => source.value === "all" || typeOptions.value.length === 0
);
const typeSelectOptions = computed(() => [
  { value: "", label: typeDisabled.value ? "选择来源后可筛选" : "全部类型" },
  ...typeOptions.value,
]);
const queryGameVersion = computed(() => {
  if (versionMode.value === "all") return null;
  if (versionMode.value === "instance") return context.value?.game_version ?? null;
  return versionMode.value;
});
const loader = computed(() => {
  if (context.value && context.value.loader !== "vanilla") {
    return context.value.loader;
  }
  if (!instanceId.value && preset?.loader && preset.loader !== "vanilla") {
    return preset.loader;
  }
  return null;
});
/// 加载器筛选只对 Mod 和整合包有意义；光影/资源包/数据包带上它会被筛成 0 条
const searchLoader = computed(() =>
  props.kind === "mods" || props.kind === "modpacks" ? loader.value : null
);
const activeSources = computed<SourceKey[]>(() =>
  source.value === "all" ? ["curseforge", "modrinth"] : [source.value]
);
const items = computed(() => {
  const list = activeSources.value.flatMap((key) => buckets.value[key].items);
  const sorted = [...list].sort((a, b) =>
    sortField.value === "3"
      ? b.updated.localeCompare(a.updated)
      : b.download_count - a.download_count
  );
  return sorted;
});
const total = computed(() =>
  activeSources.value.reduce((sum, key) => sum + buckets.value[key].total, 0)
);
const hasMore = computed(() =>
  activeSources.value.some((key) => !buckets.value[key].done)
);

onMounted(async () => {
  // 地图只有 CurseForge 收录，直接锁定来源
  if (props.kind === "worlds") source.value = "curseforge";
  const fromPreset =
    preset?.instanceId &&
    store.installed.some((v) => v.id === preset.instanceId)
      ? preset.instanceId
      : "";
  // 默认不绑定实例（仅浏览），仅当从实例页带 preset 进入时才预选
  instanceId.value = props.kind === "modpacks" ? "" : fromPreset;
  if (instanceId.value) {
    versionMode.value = "instance";
  } else if (preset?.gameVersion) {
    versionMode.value = preset.gameVersion;
  } else {
    versionMode.value = "all";
  }
  if (preset?.query) query.value = preset.query;
  loadVersions();
  void loadCategories();
  await loadContext();
  await search(true);
  // 版本 tab 需要与官方版本表求交集，确保 manifest 已加载
  if (!store.versions.length) void refreshVersions();
  if (preset?.projectId) {
    const hit =
      items.value.find((project) => project.id === preset.projectId) ??
      items.value[0];
    if (hit) openProject(hit);
  }
  store.communityContext = null;
});

watch(instanceId, async () => {
  versionMode.value = instanceId.value ? "instance" : "all";
  active.value = null;
  await loadContext();
  await search(true);
});

watch(
  () => props.kind,
  async () => {
    active.value = null;
    category.value = "";
    if (props.kind === "worlds") source.value = "curseforge";
    void loadCategories();
    await search(true);
  }
);

watch(category, () => {
  if (typeDisabled.value) return;
  void search(true);
});

async function loadVersions() {
  try {
    versions.value = await api.cfGameVersions();
  } catch {
    versions.value = [];
  }
}

/// 类型下拉数据：CurseForge 按 classId 拉分类，Modrinth 拉官方标签
async function loadCategories() {
  const tasks: Promise<void>[] = [];
  if (source.value !== "modrinth") {
    const classId = CLASS_IDS[props.kind] ?? 6;
    tasks.push(
      api
        .cfCategories(classId)
        .then((list) => {
          cfCategories.value = list;
        })
        .catch(() => {
          cfCategories.value = [];
        })
    );
  }
  if (source.value !== "curseforge") {
    tasks.push(
      api
        .modrinthCategories(props.kind)
        .then((list) => {
          mrCategories.value = list;
        })
        .catch(() => {
          mrCategories.value = [];
        })
    );
  }
  await Promise.allSettled(tasks);
}

async function loadContext() {
  context.value = null;
  if (!instanceId.value) return;
  try {
    context.value = await api.instanceContext(instanceId.value);
  } catch {
    context.value = null;
  }
}

async function search(reset = true) {
  if (loading.value) return;
  if (reset) buckets.value = emptyBuckets();
  const targets = activeSources.value.filter(
    (key) => reset || !buckets.value[key].done
  );
  if (!targets.length) return;
  loading.value = true;
  error.value = "";
  const results = await Promise.allSettled(
    targets.map((key) => fetchBucket(key, reset ? 0 : buckets.value[key].next))
  );
  const failed = results.find((result) => result.status === "rejected");
  if (failed && failed.status === "rejected") {
    error.value = String(failed.reason);
  }
  loading.value = false;
}

/// 项目中文简介（在线接口只提供简介翻译）
const zhNames = ref<Record<string, { summary: string }>>({});

/// MC 百科中文名：key 为 `source-id`
const mcNames = ref<Record<string, { name: string; url: string }>>({});

function displayName(project: CfProject) {
  return mcNames.value[`${project.source}-${project.id}`]?.name || project.name;
}

function displaySummary(project: CfProject) {
  return zhNames.value[`${project.source}-${project.id}`]?.summary || project.summary;
}

/// 没缓存过的项目去 MC 百科查中文名（后端有节流与 30 天缓存）
async function loadChineseNames(list: CfProject[]) {
  if (!list.length) return;
  const queries: { key: string; title: string; slug: string }[] = [];
  const queued = new Set<string>();
  for (const project of list) {
    const key = `${project.source}-${project.id}`;
    if (mcNames.value[key] || queued.has(key)) continue;
    // 源站本身就写中文的不用查
    if (/[\u4e00-\u9fff]/.test(project.name)) continue;
    queued.add(key);
    queries.push({ key, title: project.name, slug: project.slug || "" });
  }
  if (!queries.length) return;
  try {
    const map = await api.mcmodNames(queries);
    if (Object.keys(map).length) {
      mcNames.value = { ...mcNames.value, ...map };
    }
  } catch {
    // 查询失败保持英文名
  }
}

async function fetchBucket(key: SourceKey, offset: number) {
  const version = queryGameVersion.value;
  const selected = category.value;
  const cfCategory =
    source.value === "curseforge" && selected ? Number(selected) : null;
  const mrCategory =
    source.value === "modrinth" && selected ? selected : null;
  const result =
    key === "curseforge"
      ? await api.cfSearch(
          CLASS_IDS[props.kind] ?? 6,
          query.value,
          version,
          searchLoader.value,
          offset,
          Number(sortField.value),
          cfCategory
        )
      : await api.modrinthSearch(
          props.kind,
          query.value,
          version,
          searchLoader.value,
          offset,
          Number(sortField.value),
          mrCategory
        );
  const bucket = buckets.value[key];
  const items = [...bucket.items, ...result.items];
  buckets.value = {
    ...buckets.value,
    [key]: {
      items,
      total: result.total,
      next: offset + 20,
      done: result.items.length === 0 || items.length >= result.total,
    },
  };
  void loadTranslations(result.items);
  void loadChineseNames(result.items);
}

async function loadTranslations(list: CfProject[]) {
  if (!list.length) return;
  // 同一来源一次请求，两个来源分别取
  for (const source of ["curseforge", "modrinth"] as const) {
    const ids = list.filter((item) => item.source === source).map((item) => item.id);
    if (!ids.length) continue;
    try {
      const map = await api.projectTranslations(source, ids);
      const next = { ...zhNames.value };
      for (const [id, value] of Object.entries(map)) {
        if (value.summary) next[`${source}-${id}`] = value;
      }
      zhNames.value = next;
    } catch {
      // 翻译失败保持英文原名
    }
  }
}

function resetSearch() {
  query.value = "";
  source.value = props.kind === "worlds" ? "curseforge" : "all";
  versionMode.value = instanceId.value ? "instance" : "all";
  sortField.value = "2";
  category.value = "";
  search(true);
}

function switchSource(value: string) {
  const next = value as "all" | SourceKey;
  if (source.value === next) return;
  source.value = next;
  // 分类 id 在两个来源之间互不通用，切来源时重置
  category.value = "";
  void loadCategories();
  search(true);
}

async function loadMore() {
  if (loading.value) return;
  const targets = activeSources.value.filter(
    (key) => !buckets.value[key].done
  );
  if (!targets.length) return;
  loading.value = true;
  await Promise.allSettled(
    targets.map((key) => fetchBucket(key, buckets.value[key].next))
  );
  loading.value = false;
}

function openProject(project: CfProject) {
  if (active.value && active.value.id !== project.id) {
    projectStack.value.push(active.value);
  }
  active.value = project;
  files.value = [];
  filesError.value = "";
  depFile.value = null;
  void loadTranslations([project]);
  void loadChineseNames([project]);
  loadFiles(project);
}

/// 返回上一级项目（从前置跳进来时），没有历史就回到搜索结果。
function goBack() {
  const previous = projectStack.value.pop();
  if (!previous) {
    active.value = null;
    return;
  }
  active.value = previous;
  files.value = [];
  filesError.value = "";
  depFile.value = null;
  void loadFiles(previous);
}

/// 展开某个文件的前置列表；再次点击收起。
async function toggleDeps(file: CfFileView) {
  if (depFile.value?.id === file.id) {
    depFile.value = null;
    return;
  }
  depFile.value = file;
  depProjects.value = [];
  depError.value = "";
  if (!file.dependencies.length) return;
  depLoading.value = true;
  try {
    depProjects.value = await api.contentProjects(
      active.value?.source ?? "modrinth",
      file.dependencies
    );
  } catch (err) {
    depError.value = String(err);
  } finally {
    depLoading.value = false;
  }
}

async function loadFiles(project: CfProject) {
  filesLoading.value = true;
  depFile.value = null;
  try {
    files.value =
      project.source === "modrinth"
        ? await api.modrinthFiles(
            project.id,
            queryGameVersion.value,
            props.kind === "mods" ? loader.value : null
          )
        : await api.cfFiles(
            project.id,
            queryGameVersion.value,
            props.kind === "mods" ? loader.value : null
          );
  } catch (err) {
    filesError.value = String(err);
  } finally {
    filesLoading.value = false;
  }
}

function install(project: CfProject, file: CfFileView) {
  if (!file.download_url || !instanceId.value) return;
  if (props.kind === "worlds") {
    installWorld(
      instanceId.value,
      file.download_url,
      file.file_name,
      `${project.name} ${file.name}`
    );
    return;
  }
  installCfFile(
    instanceId.value,
    preset?.installKind ?? props.kind,
    file.download_url,
    file.file_name,
    `${project.name} ${file.name}`
  );
}

/// 进入光影包详情时自动检测实例的光影加载器，缺了就给出推荐项
async function loadShaderLoaderHint() {
  shaderLoaderHint.value = null;
  const target = instanceId.value;
  if (props.kind !== "shaders" || !target) return;
  try {
    const support = await api.shaderSupport(target);
    if (support.provider) return;
    if (support.recommend) {
      shaderLoaderHint.value = {
        name: support.recommend_label ?? support.recommend,
        summary: "装好这个光影加载器，光影包才能在当前实例生效",
        project: null,
        vanilla: null,
      };
      try {
        const list = await api.contentProjects("modrinth", [support.recommend]);
        const project =
          list.find((item) => item.id === support.recommend) ?? list[0] ?? null;
        if (project) {
          shaderLoaderHint.value = {
            name: project.name,
            summary: project.summary,
            project,
            vanilla: null,
          };
          void loadTranslations([project]);
          void loadChineseNames([project]);
        }
      } catch {
        // 取不到项目详情就保留占位推荐
      }
      return;
    }
    shaderLoaderHint.value = {
      name: "Fabric",
      summary: "原版实例需要先装 Fabric，再装 Iris 才能加载光影",
      project: null,
      vanilla: { loader: "fabric", gameVersion: support.game_version },
    };
  } catch {
    shaderLoaderHint.value = null;
  }
}

/// 点击推荐项：直接装光影加载器；原版实例只是提示，不跳转
async function installHintLoader() {
  const hint = shaderLoaderHint.value;
  const target = instanceId.value;
  if (!hint || !target || hint.vanilla) return;
  const installed = await installShaderLoader(target);
  if (installed) shaderLoaderHint.value = null;
}

function releaseTag(type: number) {
  if (type === 1) return "正式版";
  if (type === 2) return "测试版";
  if (type === 3) return "Alpha";
  return "";
}

function sourceLabel(value: string) {
  return value === "modrinth" ? "Modrinth" : "CurseForge";
}

function compactNumber(value: number) {
  if (value >= 100_000_000) return `${(value / 100_000_000).toFixed(1)} 亿`;
  if (value >= 10_000) return `${(value / 10_000).toFixed(1)} 万`;
  return String(value);
}

function updatedLabel(iso: string) {
  if (!iso) return "";
  const time = new Date(iso).getTime();
  if (Number.isNaN(time)) return "";
  const diff = Date.now() - time;
  const hour = 3_600_000;
  const day = 24 * hour;
  if (diff < hour) return "刚刚更新";
  if (diff < day) return `${Math.floor(diff / hour)} 小时前`;
  if (diff < 30 * day) return `${Math.floor(diff / day)} 天前`;
  return formatDate(iso);
}

function shortVersions(list: string[]) {
  return list.slice(0, 2).join(" / ");
}

function refresh() {
  notify("正在刷新社区资源");
  loadVersions();
  search(true);
}

async function openPreview(path: string, fromTemp: boolean) {
  inspecting.value = true;
  try {
    const preview = await api.modpackInspect(path);
    packPath.value = path;
    packPreview.value = preview;
    importForm.name = preview.name;
    importForm.id = preview.suggested_id;
    importForm.loaderVersion = preview.loader_version ?? "";
    const installedIds = store.installed.map((item) => item.id);
    importForm.parentVersion =
      preview.detected_parent && installedIds.includes(preview.detected_parent)
        ? preview.detected_parent
        : installedIds[0] ?? "";
    importForm.optional = false;
    importForm.server = false;
    showImport.value = true;
  } catch (err) {
    notify(`读取整合包失败：${String(err)}`, "err");
    if (fromTemp) api.modpackTempRemove(path).catch(() => {});
  } finally {
    inspecting.value = false;
  }
}

async function pickPack() {
  if (store.installingId) {
    notify("已有下载任务正在进行", "err");
    return;
  }
  const selection = await openDialog({
    multiple: false,
    directory: false,
    title: "选择整合包压缩包",
    filters: [{ name: "整合包", extensions: ["zip", "mrpack"] }],
  });
  const path = Array.isArray(selection) ? selection[0] : selection;
  if (!path) return;
  await openPreview(path, false);
}

/// 表单前置模式：还没下载的整合包（清单未知，确认后才开始下载）
interface PendingPack {
  url: string;
  fileName: string;
  title: string;
  name: string;
  suggestedId: string;
  sourceLabel: string;
}

const pendingPack = ref<PendingPack | null>(null);

// 表单关闭时清掉未下载的挂起状态，避免残留影响下一次打开
watch(showImport, (open) => {
  if (!open) {
    pendingPack.value = null;
    packPreview.value = null;
  }
});

/// 后台导入任务：表单确认后立即关闭，下载、解析、安装全部在后台执行，
/// 进度走下载中心，取消也在下载中心操作
interface PackImportJob {
  path: string | null;
  pending: PendingPack | null;
  options: {
    instance_name: string | null;
    instance_id: string | null;
    loader_version: string | null;
    parent_version: string | null;
    install_optional: boolean;
    server: boolean;
  };
}

async function runPackImport(job: PackImportJob) {
  let path = job.path;
  const fromTemp = job.pending != null;
  if (!path && job.pending) {
    path = await downloadModpack(job.pending.url, job.pending.fileName, job.pending.title);
    if (!path) return;
  }
  let preview: PackPreview;
  try {
    preview = await api.modpackInspect(path!);
  } catch (err) {
    notify(`读取整合包失败：${String(err)}`, "err");
    if (fromTemp) api.modpackTempRemove(path!).catch(() => {});
    return;
  }
  // 用户留空的字段用清单解析值补齐
  const options = { ...job.options };
  if (!options.loader_version) options.loader_version = preview.loader_version ?? null;
  if (preview.format === "generic") {
    const installedIds = store.installed.map((item) => item.id);
    const current = options.parent_version;
    options.parent_version =
      current && installedIds.includes(current)
        ? current
        : preview.detected_parent && installedIds.includes(preview.detected_parent)
          ? preview.detected_parent
          : installedIds[0] ?? null;
    if (!options.parent_version) {
      notify("该压缩包需要父版本，请先安装一个基础版本再导入", "err");
      if (fromTemp) api.modpackTempRemove(path!).catch(() => {});
      return;
    }
  }
  const result = await importModpack(path!, options, preview.name);
  if (result) {
    if (fromTemp) api.modpackTempRemove(path!).catch(() => {});
    notify(
      options.server
        ? `${preview.name} 服务端已就绪，可在联机页一键开服`
        : `${preview.name} 导入完成，已加入版本列表`
    );
  }
}

async function installPack(project: CfProject, file: CfFileView) {
  if (!file.download_url || inspecting.value) return;
  const slug =
    project.slug ||
    project.name
      .toLowerCase()
      .replace(/[^a-z0-9\u4e00-\u9fa5]+/g, "-")
      .replace(/^-+|-+$/g, "") ||
    project.name;
  pendingPack.value = {
    url: file.download_url,
    fileName: file.file_name,
    title: `${project.name} ${file.name}`,
    name: project.name,
    suggestedId: slug,
    sourceLabel: project.source === "curseforge" ? "CurseForge 整合包" : "Modrinth 整合包",
  };
  importForm.name = project.name;
  importForm.id = slug;
  importForm.loaderVersion = "";
  importForm.parentVersion = "";
  importForm.optional = false;
  importForm.server = false;
  showImport.value = true;
}

/// 从源站文件元数据提取游戏版本与加载器，作为服务端包导入的提示
function packHints(file: CfFileView) {
  const mc = file.game_versions.find((value) => /^\d+\.\d+(\.\d+)?$/.test(value)) ?? "";
  const lower = file.game_versions.map((value) => value.toLowerCase());
  const loader = lower.includes("neoforge")
    ? "neoforge"
    : lower.includes("forge")
      ? "forge"
      : lower.includes("fabric")
        ? "fabric"
        : "";
  return { mc, loader };
}

/// 下载服务端包并导入到服务端目录，成功后联机页可直接开服
async function importServerArchive(
  url: string,
  fileName: string,
  title: string,
  slug: string,
  gameVersion = "",
  loader = ""
) {
  const path = await downloadModpack(url, fileName, title);
  if (!path) return;
  // 解压可能耗时，占用安装标记避免并发任务冲突
  store.installingId = `服务端 · ${title}`;
  notify(`正在解压 ${title}…`);
  try {
    const pack = await api.serverPackImport(
      path,
      title,
      slug,
      gameVersion || null,
      loader || null
    );
    notify(
      pack.needs_install
        ? `${pack.name} 已导入；首次开服会自动安装 ${pack.loader}（需下载依赖）`
        : `${pack.name} 已就绪，可在联机页一键开服`
    );
  } finally {
    store.installingId = null;
    api.modpackTempRemove(path).catch(() => {});
  }
}

/// 一键下载配套服务端包（客户端包 → 指向的服务端包）
async function installServerPack(file: CfFileView) {
  const fileId = file.server_pack_file_id;
  if (!active.value || !fileId || inspecting.value || store.installingId) return;
  const project = active.value;
  inspecting.value = true;
  try {
    const link = await api.cfFileLink(project.id, fileId);
    const hints = packHints(file);
    await importServerArchive(
      link.url,
      link.file_name,
      `${project.name} 服务端`,
      `${project.slug || project.id}-server`,
      hints.mc,
      hints.loader
    );
  } catch (err) {
    notify(`下载服务端失败：${String(err)}`, "err");
  } finally {
    inspecting.value = false;
  }
}

/// 文件本身就是服务端包（作者单独上传）
async function importServerFile(file: CfFileView) {
  if (!active.value || !file.download_url || inspecting.value || store.installingId) return;
  const project = active.value;
  inspecting.value = true;
  try {
    const hints = packHints(file);
    await importServerArchive(
      file.download_url,
      file.file_name,
      `${project.name} 服务端`,
      `${project.slug || project.id}-server`,
      hints.mc,
      hints.loader
    );
  } catch (err) {
    notify(`导入服务端失败：${String(err)}`, "err");
  } finally {
    inspecting.value = false;
  }
}

/// 本地服务端整合包：直接解压到服务端目录
async function importLocalAsServer() {
  const preview = packPreview.value;
  const path = packPath.value;
  if (!preview || !path || store.installingId) return;
  store.installingId = `服务端 · ${preview.name}`;
  try {
    const pack = await api.serverPackImport(
      path,
      preview.name,
      preview.suggested_id,
      null,
      null
    );
    showImport.value = false;
    notify(
      pack.needs_install
        ? `${pack.name} 已导入；首次开服会自动安装 ${pack.loader}（需下载依赖）`
        : `${pack.name} 已就绪，可在联机页一键开服`
    );
  } catch (err) {
    notify(`导入服务端失败：${String(err)}`, "err");
  } finally {
    store.installingId = null;
  }
}

async function confirmImport() {
  if (showImport.value === false) return;
  const pending = pendingPack.value;
  const preview = packPreview.value;
  if (!pending && !preview) return;
  // 本地文件路径：已有清单，保持原校验
  if (!pending && preview?.format === "generic" && !importForm.parentVersion) {
    notify("请选择一个已安装的父版本", "err");
    return;
  }
  if (store.installingId) {
    notify("已有下载任务正在进行，请等待其完成", "err");
    return;
  }
  // 快照参数后立即关表单，任务转后台执行
  const job: PackImportJob = {
    path: pending ? null : packPath.value,
    pending,
    options: {
      instance_name: importForm.name.trim() || null,
      instance_id: importForm.id.trim() || null,
      loader_version: importForm.loaderVersion.trim() || null,
      parent_version: importForm.parentVersion || null,
      install_optional: importForm.optional,
      server: importForm.server,
    },
  };
  showImport.value = false;
  void runPackImport(job);
}
</script>

<template>
  <div class="cf">
    <section class="card card-pad cf-search">
      <div class="cf-search__head">
        <div class="cf-search__title">搜索{{ label }}</div>
        <span v-if="context" class="tag">
          {{ LOADER_LABELS[context.loader] ?? context.loader }} · {{ context.game_version }}
        </span>
      </div>

      <div class="cf-fields">
        <label v-if="sourceOptions.length > 1" class="cf-field cf-field--source">
          <span class="cf-field__label">来源</span>
          <div class="cf-source-tabs">
            <button
              v-for="option in sourceOptions"
              :key="option.value"
              class="cf-source-tab"
              :class="{ 'cf-source-tab--on': source === option.value }"
              @click="switchSource(option.value)"
            >
              {{ option.label }}
            </button>
          </div>
        </label>

        <label class="cf-field cf-field--wide">
          <span class="cf-field__label">名称</span>
          <WoodInput
            v-model="query"
            :placeholder="`输入${label}名称，留空浏览全部`"
            @keyup.enter="search(true)"
          />
        </label>

        <label class="cf-field">
          <span class="cf-field__label">版本</span>
          <WoodSelect v-model="versionMode" :options="versionOptions" />
        </label>

        <label class="cf-field">
          <span class="cf-field__label">排序</span>
          <WoodSelect v-model="sortField" :options="sortOptions" />
        </label>

        <label class="cf-field">
          <span class="cf-field__label">目标实例</span>
          <span v-if="kind === 'modpacks'" class="cf-field__fallback muted small">
            导入时自动创建
          </span>
          <WoodSelect
            v-else-if="store.installed.length"
            v-model="instanceId"
            :options="instanceOptions"
          />
          <span v-else class="cf-field__fallback muted small">暂未安装版本</span>
        </label>

        <label class="cf-field cf-field--type">
          <span class="cf-field__label">类型</span>
          <WoodSelect
            v-model="category"
            :options="typeSelectOptions"
            :disabled="typeDisabled"
          />
        </label>
      </div>

      <div class="cf-actions">
        <WoodButton variant="primary" sheen :disabled="loading" @click="search(true)">
          <AppIcon name="grid" :size="15" />
          搜索
        </WoodButton>
        <WoodButton variant="ghost" @click="resetSearch">重置条件</WoodButton>
        <WoodButton
          v-if="kind === 'modpacks'"
          variant="soft"
          :disabled="inspecting || !!store.installingId"
          @click="pickPack"
        >
          <AppIcon name="package" :size="15" />
          {{ inspecting ? "读取中…" : "导入整合包" }}
        </WoodButton>
        <IconButton
          icon="refresh"
          label="刷新列表"
          variant="soft"
          :size="38"
          @click="refresh"
        />
        <span v-if="!instanceOptions.length && kind !== 'modpacks'" class="muted small cf-actions__hint">
          还没安装版本也可以浏览；下载文件前需要先在「原版游戏」里安装一个实例。
        </span>
      </div>
    </section>

    <template v-if="active">
      <div class="cf-detail-head">
        <IconButton
          icon="chevron-left"
          label="返回上一级"
          :size="34"
          @click="goBack"
        />
        <img v-if="active.logo_url" class="cf-detail__logo" :src="active.logo_url" alt="" />
        <div class="cf-detail__text">
          <div class="cf-detail__title-row">
            <h2 class="cf-detail__title">{{ displayName(active) }}</h2>
            <span class="tag">{{ sourceLabel(active.source) }}</span>
            <span
              v-for="category in active.categories.slice(0, 3)"
              :key="category"
              class="chip"
            >
              {{ zhCategory(category) }}
            </span>
          </div>
          <p v-if="displayName(active) !== active.name" class="muted small cf-detail__origin">
            {{ active.name }}
          </p>
          <p class="muted small">
            {{ active.author || "未知作者" }} · {{ compactNumber(active.download_count) }} 次下载
            <template v-if="active.updated"> · {{ updatedLabel(active.updated) }}</template>
          </p>
          <p class="muted small cf-detail__summary">{{ displaySummary(active) }}</p>
          <div class="cf-detail__links">
            <button
              v-for="link in projectLinks"
              :key="link.label"
              class="cf-link"
              @click="openExternal(link.url)"
            >
              <AppIcon name="external" :size="13" />
              {{ link.label }}
            </button>
            <button class="cf-link" @click="copyProjectName">
              <AppIcon name="copy" :size="13" />
              复制名称
            </button>
          </div>
        </div>
      </div>

      <div v-if="shaderLoaderHint" class="card card-pad shader-loader reveal">
        <span class="cf-deps__title">光影加载器</span>
        <div class="cf-deps__list">
          <component
            :is="shaderLoaderHint.vanilla ? 'div' : 'button'"
            class="cf-dep"
            :class="{ 'cf-dep--static': shaderLoaderHint.vanilla }"
            :type="shaderLoaderHint.vanilla ? undefined : 'button'"
            @click="installHintLoader"
          >
            <img
              v-if="shaderLoaderHint.project?.logo_url"
              class="cf-dep__logo"
              :src="shaderLoaderHint.project.logo_url"
              alt=""
            />
            <span v-else class="cf-dep__logo cf-dep__logo--empty" />
            <div class="cf-dep__text">
              <strong>
                {{
                  shaderLoaderHint.project
                    ? displayName(shaderLoaderHint.project)
                    : shaderLoaderHint.name
                }}
              </strong>
              <span>
                {{
                  shaderLoaderHint.project
                    ? displaySummary(shaderLoaderHint.project)
                    : shaderLoaderHint.summary
                }}
              </span>
            </div>
            <span class="tag">{{ shaderLoaderHint.vanilla ? "加载器" : "Modrinth" }}</span>
            <AppIcon
              v-if="!shaderLoaderHint.vanilla"
              class="shader-loader__go"
              name="download"
              :size="14"
            />
          </component>
        </div>
      </div>

      <section class="card card-pad cf-files">
        <div class="row-between">
          <div>
            <div class="section-title">可用文件</div>
            <p class="muted small">
              {{ queryGameVersion ? `按 ${queryGameVersion} 筛选` : "全部版本" }}
              <template v-if="kind === 'mods' && loader">
                · {{ LOADER_LABELS[loader] }}
              </template>
              <template v-if="kind === 'modpacks'"> · 点下载直接安装成新实例</template>
            </p>
          </div>
        </div>

        <div v-if="filesLoading" class="cf-files__state">
          <span class="skeleton" />
          <span class="muted small">正在获取文件列表…</span>
        </div>
        <div v-else-if="filesError" class="cf-files__state cf-files__state--error">
          {{ filesError }}
        </div>
        <div v-else-if="!files.length" class="cf-files__state muted small">
          没有找到适配当前筛选的文件。
        </div>
        <template v-else>
          <div v-if="versionTabs.length > 2" class="cf-version-tabs">
            <button
              v-for="version in versionTabs"
              :key="version"
              class="cf-version-tab"
              :class="{ 'cf-version-tab--active': versionTab === version }"
              @click="versionTab = version"
            >
              {{ version === "all" ? "全部" : version }}
            </button>
          </div>
          <div v-else class="cf-version-tabs cf-version-tabs--empty" />
          <div class="cf-file-list">
            <div v-for="file in visibleFiles" :key="file.id" class="cf-file">
            <div class="cf-file__main">
              <span class="cf-file__name">{{ file.name }}</span>
              <span class="cf-file__meta mono">
                {{ shortVersions(file.game_versions) }}
                <template v-if="file.game_versions.length > 2">…</template>
              </span>
            </div>
            <span v-if="releaseTag(file.release_type)" class="tag">
              {{ releaseTag(file.release_type) }}
            </span>
            <span v-if="file.is_server_pack" class="tag tag--grass">服务端</span>
            <button
              v-if="kind === 'modpacks' && file.is_server_pack && file.download_url"
              class="cf-file__deps"
              title="解压到服务端目录，可在联机页开服"
              :disabled="inspecting || !!store.installingId"
              @click="importServerFile(file)"
            >
              导入服务端
            </button>
            <button
              v-else-if="kind === 'modpacks' && file.server_pack_file_id"
              class="cf-file__deps"
              title="下载该整合包配套的服务端包"
              :disabled="inspecting || !!store.installingId"
              @click="installServerPack(file)"
            >
              下载服务端
            </button>
            <button
              v-if="file.dependencies.length"
              class="cf-file__deps"
              :class="{ 'cf-file__deps--open': depFile?.id === file.id }"
              :title="`查看 ${file.dependencies.length} 项前置`"
              @click="toggleDeps(file)"
            >
              前置 {{ file.dependencies.length }}
            </button>
            <span class="cf-file__size mono">{{ formatBytes(file.size) }}</span>
            <span class="cf-file__date mono">{{ formatDate(file.file_date) }}</span>
            <IconButton
              v-if="file.download_url && kind === 'modpacks' && !file.is_server_pack"
              icon="download"
              :label="`安装 ${file.name}`"
              variant="soft"
              :size="30"
              :disabled="!!store.installingId || inspecting"
              @click="installPack(active, file)"
            />
            <IconButton
              v-else-if="file.download_url && instanceId"
              icon="download"
              :label="`下载 ${file.name}`"
              variant="soft"
              :size="30"
              :disabled="!!store.installingId"
              @click="install(active, file)"
            />
            <span v-else-if="!file.download_url" class="muted small cf-file__hint">
              无下载地址
            </span>
            <span v-else class="muted small cf-file__hint">先安装一个实例</span>

            <div v-if="depFile?.id === file.id" class="cf-deps">
              <span v-if="depLoading" class="cf-deps__state muted small">正在读取前置资源…</span>
              <span v-else-if="depError" class="cf-deps__state muted small">{{ depError }}</span>
              <span v-else-if="!depProjects.length" class="cf-deps__state muted small">
                没有取到前置详情，安装时会自动尝试补齐。
              </span>
              <template v-else>
                <span class="cf-deps__title">前置资源</span>
                <div class="cf-deps__list">
                  <button
                    v-for="dep in depProjects"
                    :key="dep.id"
                    type="button"
                    class="cf-dep"
                    :title="`查看 ${displayName(dep)}`"
                    @click="openProject(dep)"
                  >
                    <img v-if="dep.logo_url" :src="dep.logo_url" alt="" class="cf-dep__logo" />
                    <span v-else class="cf-dep__logo cf-dep__logo--empty" />
                    <div class="cf-dep__text">
                      <strong>{{ displayName(dep) }}</strong>
                      <span>{{ displaySummary(dep) }}</span>
                    </div>
                    <span class="tag">
                      {{ dep.source === "modrinth" ? "Modrinth" : "CurseForge" }}
                    </span>
                    <AppIcon class="cf-dep__go" name="chevron-left" :size="12" />
                  </button>
                </div>
              </template>
            </div>
          </div>
        </div>
        </template>
      </section>
    </template>

    <template v-else>
      <div v-if="error && !items.length" class="card card-pad cf-state cf-state--error">
        <span>{{ error }}</span>
        <WoodButton variant="ghost" size="sm" @click="search(true)">重试</WoodButton>
      </div>

      <div v-else-if="loading && !items.length" class="cf-list">
        <div v-for="n in 8" :key="n" class="cf-row cf-row--skeleton">
          <span class="skeleton cf-row__logo" />
          <div class="cf-row__body">
            <span class="skeleton cf-skeleton__title" />
            <span class="skeleton cf-skeleton__line" />
          </div>
        </div>
      </div>

      <div v-else-if="!items.length" class="card card-pad cf-state">
        <AppIcon name="package" :size="30" />
        <div>
          <p class="cf-state__title">没有匹配的{{ label }}</p>
          <p class="muted small">换个关键词，或者把版本切换为「全部版本」再试。</p>
        </div>
      </div>

      <div v-else class="cf-list">
        <button
          v-for="(project, i) in items"
          :key="`${project.source}-${project.id}`"
          class="cf-row"
          :style="{ '--i': i }"
          @click="openProject(project)"
        >
          <img
            v-if="project.logo_url"
            class="cf-row__logo"
            :src="project.logo_url"
            alt=""
            loading="lazy"
          />
          <span v-else class="cf-row__logo cf-row__logo--empty">
            {{ project.name.slice(0, 1) }}
          </span>

          <div class="cf-row__body">
            <div class="cf-row__title">
              <strong class="cf-row__name">{{ displayName(project) }}</strong>
              <span
                v-if="displayName(project) !== project.name"
                class="cf-row__origin muted small"
              >
                {{ project.name }}
              </span>
              <span class="cf-row__source">{{ sourceLabel(project.source) }}</span>
              <span
                v-for="category in project.categories.slice(0, 2)"
                :key="category"
                class="chip"
              >
                {{ zhCategory(category) }}
              </span>
            </div>
            <p class="cf-row__summary">{{ displaySummary(project) }}</p>
          </div>

          <div class="cf-row__meta">
            <span class="cf-row__stat cf-row__stat--author">
              <AppIcon name="user" :size="12" />
              <span>{{ project.author || "未知作者" }}</span>
            </span>
            <span class="cf-row__stat cf-row__stat--count">
              <AppIcon name="download" :size="12" />
              <span>{{ compactNumber(project.download_count) }}</span>
            </span>
            <span v-if="project.updated" class="cf-row__stat cf-row__stat--date">
              <AppIcon name="refresh" :size="12" />
              <span>{{ updatedLabel(project.updated) }}</span>
            </span>
          </div>
        </button>
      </div>

      <div v-if="items.length && hasMore" class="load-more">
        <WoodButton variant="ghost" size="sm" :disabled="loading" @click="loadMore">
          <AppIcon name="chevron-down" :size="14" />
          加载更多（还有 {{ Math.max(total - items.length, 0) }} 个）
        </WoodButton>
      </div>
    </template>

    <Teleport to="body">
      <Transition name="pack-modal">
        <div v-if="showImport && (packPreview || pendingPack)" class="pack-modal">
          <div class="pack-modal__mask" @click="showImport = false" />
          <div class="pack-modal__card">
            <header class="pack-modal__head">
              <div>
                <span class="section-title">整合包</span>
                <h2 class="pack-modal__title">{{ pendingPack?.name ?? packPreview?.name }}</h2>
                <p class="muted small">
                  <template v-if="pendingPack">
                    {{ pendingPack.sourceLabel }}
                    <template v-if="packPreview"> · {{ packPreview.version }}</template>
                  </template>
                  <template v-else-if="packPreview">
                    {{
                      packPreview.format === "curseforge"
                        ? "CurseForge 整合包"
                        : packPreview.format === "mrpack"
                          ? "Modrinth 整合包"
                          : "通用压缩包"
                    }}
                    <template v-if="packPreview.version"> · {{ packPreview.version }}</template>
                    <template v-if="packPreview.game_version"> · {{ packPreview.game_version }}</template>
                  </template>
                </p>
              </div>
              <IconButton icon="close" label="关闭" :size="32" @click="showImport = false" />
            </header>

            <div class="pack-modal__body">
              <div class="pack-summary">
                <template v-if="pendingPack">
                  <span class="tag">清单将在下载后解析</span>
                  <span class="tag">{{ pendingPack.fileName }}</span>
                </template>
                <template v-else-if="packPreview">
                  <span class="tag">{{ packPreview.file_count }} 个文件</span>
                  <span v-if="packPreview.download_bytes" class="tag">
                    下载 {{ formatBytes(packPreview.download_bytes) }}
                  </span>
                  <span v-if="packPreview.override_bytes" class="tag">
                    覆盖文件 {{ formatBytes(packPreview.override_bytes) }}
                  </span>
                  <span class="tag">
                    {{
                      packPreview.loader === "vanilla"
                        ? "原版"
                        : packPreview.loader.charAt(0).toUpperCase() + packPreview.loader.slice(1)
                    }}
                    <template v-if="packPreview.loader_version">
                      {{ packPreview.loader_version }}
                    </template>
                  </span>
                </template>
              </div>

              <label class="pack-field">
                <span class="pack-field__label">实例名称</span>
                <WoodInput v-model="importForm.name" placeholder="显示在版本列表里的名字" />
              </label>

              <label class="pack-field">
                <span class="pack-field__label">实例 ID</span>
                <WoodInput v-model="importForm.id" mono placeholder="用于版本目录名" />
              </label>

              <label
                v-if="pendingPack || (packPreview && packPreview.loader !== 'vanilla')"
                class="pack-field"
              >
                <span class="pack-field__label">加载器版本</span>
                <WoodInput v-model="importForm.loaderVersion" mono placeholder="留空自动选择" />
              </label>

              <label v-if="packPreview?.format === 'generic'" class="pack-field">
                <span class="pack-field__label">父版本</span>
                <WoodSelect
                  v-if="parentOptions.length"
                  v-model="importForm.parentVersion"
                  :options="parentOptions"
                />
                <span v-else class="muted small pack-field__empty">
                  还没有已安装版本，请先去原版游戏安装一个。
                </span>
              </label>

              <div v-if="pendingPack || packPreview?.format !== 'generic'" class="pack-toggle">
                <div>
                  <strong>安装可选文件</strong>
                  <span>清单中标记为可选的 Mod，体积可能较大</span>
                </div>
                <WoodToggle v-model="importForm.optional" />
              </div>

              <div v-if="pendingPack || packPreview?.format !== 'generic'" class="pack-toggle">
                <div>
                  <strong>安装为服务端</strong>
                  <span>跳过仅客户端可用的 Mod，装好后可在联机页一键开服</span>
                </div>
                <WoodToggle v-model="importForm.server" />
              </div>

              <ul v-if="packPreview?.warnings.length" class="pack-warnings">
                <li v-for="warning in packPreview.warnings" :key="warning">{{ warning }}</li>
              </ul>

              <div v-if="packPreview?.server_like" class="pack-server-hint">
                <div>
                  <strong>这看起来是服务端整合包</strong>
                  <span>
                    直接导入到服务端目录即可开服，不需要选择父版本；装成实例也能玩单机。
                  </span>
                </div>
                <WoodButton
                  variant="soft"
                  size="sm"
                  :disabled="!!store.installingId"
                  @click="importLocalAsServer"
                >
                  导入为服务端
                </WoodButton>
              </div>
            </div>

            <footer class="pack-modal__foot">
              <span class="muted small">
                {{
                  pendingPack
                    ? "确认后表单即关闭，下载与安装在后台进行，进度见右上角下载中心。"
                    : "导入在后台进行，进度见下载中心，完成后会通知你。"
                }}
              </span>
              <div class="row">
                <WoodButton variant="ghost" @click="showImport = false">取消</WoodButton>
                <WoodButton
                  variant="primary"
                  sheen
                  :disabled="packPreview?.format === 'generic' && !importForm.parentVersion"
                  @click="confirmImport"
                >
                  <AppIcon name="download" :size="15" />
                  {{ pendingPack ? "下载并导入" : "开始导入" }}
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
.cf {
  display: flex;
  flex-direction: column;
  gap: 16px;
  container-type: inline-size;
}

/* 搜索面板 */
.cf-search {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.cf-search__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.cf-search__title {
  font-size: 15px;
  font-weight: 800;
  color: var(--walnut-deep);
}

.cf-fields {
  display: grid;
  grid-template-columns: minmax(230px, 1fr) minmax(0, 1fr) minmax(0, 1fr);
  gap: 12px;
}

/* 类型筛选独占第三行，宽度与单个下拉对齐 */
.cf-field--type {
  max-width: 320px;
}

.cf-field {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
}

.cf-field--wide {
  grid-column: span 2;
}

.cf-field__label {
  font-size: 11.5px;
  color: var(--muted);
  letter-spacing: 0.04em;
}

.cf-field__fallback {
  display: flex;
  align-items: center;
  height: 38px;
}

.cf-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.cf-actions__hint {
  margin-left: 4px;
}

/* 来源切换 */
.cf-source-tabs {
  display: flex;
  align-items: center;
  gap: 2px;
  height: 38px;
  padding: 3px;
  border: 1px solid var(--line);
  border-radius: 2px;
  background: var(--paper-2);
}

/* 分段按键：未选中平面，悬停带浮雕边浮起，选中压进凹槽 */
.cf-source-tab {
  --face: var(--panel-2);
  flex: 1;
  height: 30px;
  padding: 0 8px;
  border-radius: 2px;
  border-style: solid;
  border-width: 1px 1px 2px;
  border-color: transparent;
  font-size: 11.5px;
  font-weight: 700;
  color: var(--ink-soft);
  white-space: nowrap;
  transition: background 150ms ease, color 150ms ease,
    border-color 150ms ease, transform 100ms ease, box-shadow 100ms ease;
}

.cf-source-tab:not(.cf-source-tab--on):hover {
  color: var(--walnut-deep);
  background: var(--hover);
  transform: translateY(-1px);
  border-top-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-left-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-right-color: color-mix(in srgb, var(--face) 42%, #000);
  border-bottom-color: color-mix(in srgb, var(--face) 42%, #000);
}

.cf-source-tab:active {
  transform: translateY(1px);
  border-top-color: color-mix(in srgb, var(--face) 42%, #000);
  border-left-color: color-mix(in srgb, var(--face) 42%, #000);
  border-right-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-bottom-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-bottom-width: 1px;
  box-shadow: inset 0 2px 3px rgba(0, 0, 0, 0.18);
}

.cf-source-tab--on {
  background: var(--panel-2);
  color: var(--walnut-deep);
  transform: translateY(1px);
  border-top-color: color-mix(in srgb, var(--face) 42%, #000);
  border-left-color: color-mix(in srgb, var(--face) 42%, #000);
  border-right-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-bottom-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-bottom-width: 1px;
  box-shadow: inset 0 2px 3px rgba(0, 0, 0, 0.18);
}

/* 结果列表 */
.cf-list {
  display: flex;
  flex-direction: column;
  gap: 1px;
  border: 2px solid var(--frame);
  border-radius: 2px;
  background: var(--line);
  overflow: hidden;
}

.cf-row {
  display: grid;
  grid-template-columns: 48px minmax(0, 1fr) 296px;
  align-items: center;
  gap: 14px;
  width: 100%;
  padding: 11px 16px;
  text-align: left;
  color: var(--ink);
  background: var(--panel);
  transition: background var(--dur-1) ease;
  animation: rise-in 380ms var(--ease-out-expo) both;
  animation-delay: calc(var(--i, 0) * 14ms);
}

.cf-row:hover {
  background: rgba(var(--accent-rgb), 0.05);
}

.cf-row__logo {
  width: 48px;
  height: 48px;
  border-radius: 2px;
  object-fit: cover;
  background: var(--paper-2);
}

.cf-row__logo--empty {
  display: grid;
  place-items: center;
  font-family: var(--font-display);
  font-size: 20px;
  color: var(--moss);
  background: var(--moss-soft);
}

.cf-row__body {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.cf-row__title {
  display: flex;
  align-items: center;
  gap: 7px;
  min-width: 0;
}

.cf-row__origin {
  min-width: 0;
  max-width: 40%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 11.5px;
  opacity: 0.75;
}

.cf-detail__origin {
  font-family: var(--font-mono);
  font-size: 11.5px;
  opacity: 0.8;
}

.cf-row__name {
  min-width: 0;
  font-size: 14px;
  color: var(--walnut-deep);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cf-row__source {
  flex-shrink: 0;
  font-family: var(--font-mono);
  font-size: 9.5px;
  letter-spacing: 0.06em;
  color: var(--muted);
  padding: 2px 5px;
  border-radius: 4px;
  border: 1px solid var(--line);
}

.cf-row__summary {
  min-width: 0;
  font-size: 11.5px;
  line-height: 1.45;
  color: var(--ink-soft);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.chip {
  flex-shrink: 0;
  font-size: 10px;
  line-height: 1;
  padding: 3px 5px;
  border-radius: 0;
  color: var(--moss);
  background: var(--moss-soft);
  white-space: nowrap;
}

.cf-row__meta {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 72px 86px;
  align-items: center;
  gap: 10px;
  font-size: 10.5px;
  color: var(--muted);
}

.cf-row__stat {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  min-width: 0;
  white-space: nowrap;
}

.cf-row__stat--author {
  overflow: hidden;
}

.cf-row__stat--author > span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cf-row__stat--count,
.cf-row__stat--date {
  justify-content: flex-end;
  font-family: var(--font-mono);
}

.cf-row--skeleton {
  pointer-events: none;
  animation: none;
}

.cf-skeleton__title {
  width: 150px;
  height: 13px;
}

.cf-skeleton__line {
  width: 62%;
  height: 10px;
}

/* 空态 / 错误 */
.cf-state {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 34px 26px;
  color: var(--walnut);
}

.cf-state--error {
  color: var(--danger);
}

.cf-state__title {
  font-size: 15px;
  font-weight: 800;
  color: var(--walnut-deep);
  margin-bottom: 3px;
}

/* 详情 */
.cf-detail-head {
  display: flex;
  align-items: flex-start;
  gap: 13px;
}

.cf-detail__logo {
  width: 54px;
  height: 54px;
  border-radius: 2px;
  object-fit: cover;
  flex-shrink: 0;
  background: var(--paper-2);
}

.cf-detail__text {
  min-width: 0;
}

.cf-detail__title-row {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.cf-detail__title {
  font-family: var(--font-display);
  font-size: 30px;
  line-height: 1.1;
  color: var(--walnut-deep);
}

.cf-detail__summary {
  margin-top: 4px;
  max-width: 720px;
}

.cf-detail__links {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 10px;
}

.cf-link {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  --face: var(--panel-2);
  height: 30px;
  padding: 0 12px;
  border-style: solid;
  border-width: 1px 1px 2px;
  border-top-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-left-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-right-color: color-mix(in srgb, var(--face) 72%, #000);
  border-bottom-color: color-mix(in srgb, var(--face) 72%, #000);
  border-radius: 2px;
  background: var(--face);
  color: var(--ink-soft);
  font-size: 12px;
  transition: border-color var(--dur-1) ease, color var(--dur-1) ease,
    background var(--dur-1) ease, transform var(--dur-1) ease,
    box-shadow var(--dur-1) ease;
}

.cf-link:hover {
  --face: var(--hover);
  background: var(--face);
  color: var(--walnut-deep);
  transform: translateY(-1px);
}

.cf-link:active {
  transform: translateY(1px);
  border-top-color: color-mix(in srgb, var(--face) 72%, #000);
  border-left-color: color-mix(in srgb, var(--face) 72%, #000);
  border-right-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-bottom-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-bottom-width: 1px;
  box-shadow: inset 0 2px 3px rgba(0, 0, 0, 0.18);
}

.cf-version-tabs {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 4px;
  padding: 8px 10px;
  /* 筛选条做成和分段控件一样的浅凹槽 */
  border: 1px solid var(--line);
  border-radius: 2px;
  background: var(--paper-2);
  box-shadow: inset 0 1px 2px rgba(40, 28, 16, 0.08);
}

.cf-version-tabs--empty {
  border: none;
  background: transparent;
  padding: 0;
  min-height: 2px;
}

.cf-version-tab {
  height: 26px;
  padding: 0 11px;
  border-radius: 2px;
  color: var(--muted);
  font-size: 12px;
  transition: background var(--dur-1) ease, color var(--dur-1) ease,
    box-shadow var(--dur-1) ease;
}

.cf-version-tab:hover {
  color: var(--walnut-deep);
  background: rgba(var(--accent-rgb), 0.08);
}

/* 选中项压进凹槽，和分段按键、导航选中态保持同一种读法 */
.cf-version-tab--active,
.cf-version-tab--active:hover {
  color: var(--moss);
  background: var(--moss-soft);
  font-weight: 700;
  box-shadow: inset 0 1px 2px rgba(0, 0, 0, 0.1),
    0 0 0 1px color-mix(in srgb, var(--moss) 26%, transparent);
}

.cf-files {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.cf-files__state {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 12px 4px;
  color: var(--muted);
  font-size: 12px;
}

.cf-files__state .skeleton {
  width: 90px;
  height: 12px;
}

.cf-files__state--error {
  color: var(--danger);
}

.cf-file-list {
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.cf-file {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 10px;
  padding: 9px 11px;
  border-radius: 0;
  transition: background var(--dur-1) ease, box-shadow var(--dur-1) ease;
}

.cf-file:hover {
  background: var(--hover);
  box-shadow: inset 3px 0 0 var(--moss);
}

.cf-file__main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.cf-file__name {
  font-size: 13px;
  color: var(--ink);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cf-file__meta {
  font-size: 10.5px;
  color: var(--muted);
}

.cf-file__size,
.cf-file__date {
  font-size: 10.5px;
  color: var(--muted);
  flex-shrink: 0;
}

.cf-file__size {
  width: 72px;
  text-align: right;
}

.cf-file__date {
  width: 78px;
  text-align: right;
}

.cf-file__hint {
  font-size: 11px;
}

.cf-file__deps {
  flex-shrink: 0;
  height: 21px;
  padding: 0 8px;
  border-radius: 0;
  border: 1px solid var(--line-strong);
  background: var(--panel-2);
  color: var(--muted);
  font-size: 10.5px;
  font-weight: 700;
  transition: border-color var(--dur-1) ease, color var(--dur-1) ease,
    background var(--dur-1) ease;
}

.cf-file__deps:hover,
.cf-file__deps--open {
  border-color: var(--moss);
  color: var(--moss);
  background: var(--moss-soft);
}

.cf-deps {
  flex: 0 0 100%;
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin: 2px 0 4px;
  padding: 10px 12px;
  /* 依赖信息是附属内容，做成凹槽嵌在行内 */
  border-radius: 2px;
  border-style: solid;
  border-width: 1px 1px 2px;
  border-top-color: color-mix(in srgb, var(--line-strong) 45%, var(--ink));
  border-left-color: color-mix(in srgb, var(--line-strong) 45%, var(--ink));
  border-right-color: color-mix(in srgb, var(--line-strong) 58%, #fff);
  border-bottom-color: color-mix(in srgb, var(--line-strong) 58%, #fff);
  background: var(--panel-2);
  box-shadow: inset 0 1px 3px rgba(40, 28, 16, 0.1);
}

.shader-loader {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.shader-loader__go {
  flex-shrink: 0;
  color: var(--muted);
}

.cf-dep--static {
  cursor: default;
}

.cf-dep--static:hover {
  background: transparent;
  border-color: transparent;
}

.cf-deps__title {
  font-size: 10px;
  letter-spacing: 0.12em;
  color: var(--muted);
  font-family: var(--font-mono);
}

.cf-deps__state {
  font-size: 11px;
}

.cf-deps__list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.cf-dep {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 5px 6px;
  margin: 0 -6px;
  border-radius: 3px;
  border: 1px solid transparent;
  background: transparent;
  text-align: left;
  cursor: pointer;
  transition: background var(--dur-1) ease, border-color var(--dur-1) ease;
}

.cf-dep:hover {
  background: var(--panel);
  border-color: var(--line);
}

.cf-dep__go {
  flex-shrink: 0;
  color: var(--muted);
  transform: scaleX(-1);
}

.cf-dep__logo {
  width: 30px;
  height: 30px;
  border-radius: 0;
  border: 1px solid var(--line-strong);
  object-fit: cover;
  background: var(--panel);
  flex-shrink: 0;
}

.cf-dep__logo--empty {
  display: block;
}

.cf-dep__text {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.cf-dep__text strong {
  font-size: 12px;
  color: var(--ink);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cf-dep__text span {
  font-size: 10.5px;
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 导入整合包弹层 */
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
  width: min(560px, calc(100vw - 56px));
  max-height: calc(100vh - 72px);
  overflow-y: auto;
  padding: 20px 22px;
  border-radius: 2px;
  border: 2px solid var(--frame);
  border-top-color: color-mix(in srgb, var(--frame) 66%, #fff);
  border-left-color: color-mix(in srgb, var(--frame) 66%, #fff);
  border-right-color: color-mix(in srgb, var(--frame) 74%, #000);
  border-bottom-color: color-mix(in srgb, var(--frame) 74%, #000);
  background: rgba(var(--glass-rgb), 0.97);
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--panel) 46%, #fff),
    var(--shadow-2);
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

.pack-summary {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
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

.pack-field__empty {
  display: flex;
  align-items: center;
  height: 38px;
}

.pack-toggle {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 10px 0;
  border-top: 1px dashed var(--line);
  border-bottom: 1px dashed var(--line);
}

.pack-toggle strong {
  display: block;
  font-size: 13px;
  color: var(--walnut-deep);
}

.pack-toggle span {
  font-size: 11px;
  color: var(--muted);
}

.pack-warnings {
  margin: 0;
  padding: 8px 12px 8px 26px;
  border-radius: 0;
  background: rgba(var(--accent-rgb), 0.06);
  box-shadow: inset 0 1px 2px rgba(60, 40, 20, 0.06);
  color: var(--ink-soft);
  font-size: 11.5px;
  line-height: 1.6;
}

.pack-server-hint {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-top: 12px;
  padding: 10px 12px;
  border: 1px solid #d3dfc8;
  border-radius: 4px;
  background: var(--moss-soft);
}

.pack-server-hint > div {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.pack-server-hint strong {
  font-size: 12.5px;
  color: var(--moss);
}

.pack-server-hint span {
  font-size: 11.5px;
  color: var(--ink-soft);
  line-height: 1.6;
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
  max-width: 260px;
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

@container (max-width: 800px) {
  .cf-row {
    grid-template-columns: 44px minmax(0, 1fr) 196px;
  }

  .cf-row__meta {
    grid-template-columns: minmax(0, 1fr) 68px;
  }

  .cf-row__stat--date {
    display: none;
  }

  .cf-file__date {
    display: none;
  }
}

@container (max-width: 720px) {
  .cf-fields {
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  }

  .cf-field--source,
  .cf-field--wide {
    grid-column: span 2;
  }
}

@container (max-width: 560px) {
  .cf-row {
    grid-template-columns: 44px minmax(0, 1fr);
    row-gap: 6px;
  }

  .cf-row__logo {
    width: 44px;
    height: 44px;
  }

  .cf-row__meta {
    grid-column: 2;
    grid-template-columns: auto auto auto;
    justify-content: flex-start;
    gap: 14px;
  }

  .cf-row__stat--date {
    display: inline-flex;
  }
}
</style>
