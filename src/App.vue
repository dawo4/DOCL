<script setup lang="ts">
import { computed, markRaw, onBeforeUnmount, onMounted, ref, watch } from "vue";
import type { CSSProperties } from "vue";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import TitleBar from "./components/TitleBar.vue";
import DownloadCenter from "./components/DownloadCenter.vue";
import LogPanel from "./components/LogPanel.vue";
import CrashDialog from "./components/CrashDialog.vue";
import DropImportDialog from "./components/DropImportDialog.vue";
import AppIcon from "./components/AppIcon.vue";
import HomeView from "./views/HomeView.vue";
import VersionsView from "./views/VersionsView.vue";
import DownloadView from "./views/DownloadView.vue";
import AccountsView from "./views/AccountsView.vue";
import SkinsView from "./views/SkinsView.vue";
import InstanceView from "./views/InstanceView.vue";
import MultiplayerView from "./views/MultiplayerView.vue";
import SettingsView from "./views/SettingsView.vue";
import WoodModal from "./components/WoodModal.vue";
import WoodButton from "./components/WoodButton.vue";
import { api, type DroppedItem } from "./api";
import {
  applyTheme,
  initApp,
  launchServer,
  launchVersion,
  loadBackground,
  notify,
  startupUpdate,
  store,
  type Page,
} from "./store";

const dropActive = ref(false);
const dropItems = ref<DroppedItem[]>([]);
let unlistenDrop: (() => void) | undefined;

/// 开发模式下 F1-F7 切页面、F8 开关日志，供截图脚本按固定顺序批量采集
const devPages: Page[] = [
  "home",
  "versions",
  "download",
  "multiplayer",
  "accounts",
  "skins",
  "settings",
];

/// 演示日志，仅用于截图/录屏时填充日志面板
function seedDemoLog() {
  const demo = [
    ["system", "[DOCL] 正在启动 终焉决斗NW公测版"],
    ["stdout", "[main/INFO] Loading Minecraft 1.20.1 with Forge 47.2.0"],
    ["stdout", "[main/INFO] Java 17.0.9, 64-bit, Microsoft Windows 11"],
    ["stdout", "[main/INFO] Allocated 6.0 GB of memory"],
    ["stdout", "[modlauncher/INFO] Found 42 mods in mods folder"],
    ["stdout", "[main/INFO] Preparing spawn area: 87%"],
    ["stdout", "[Render thread/INFO] OpenGL 4.6.0 NVIDIA 552.44"],
    ["stderr", "[Render thread/ERROR] Missing texture: assets/docl/textures/gui/missing.png"],
    ["stdout", "[Render thread/INFO] Created: 1024x512x4 minecraft:textures/atlas/blocks.png"],
    ["stdout", "[main/INFO] Sound engine started"],
    ["stdout", "[Render thread/INFO] Narrator library for x64 successfully loaded"],
    ["system", "[DOCL] 游戏进程已就绪 (PID 12488)"],
  ] as const;
  store.log = demo.map(([stream, line]) => ({ stream, line }));
}

/// 截图脚本用的「主题 + 底图」组合：Shift+F1..F5 / Shift+F8 切换，Shift+F7 还原成用户自己的配置
const devLooks = [
  { theme: "paper", bg: "forest" },
  { theme: "terracotta", bg: "dune" },
  { theme: "nocturne", bg: "night" },
  { theme: "sakura", bg: "sakura" },
  { theme: "ocean", bg: "ocean" },
  { theme: "forge", bg: "aurora" },
];

function applyDevLook(event: KeyboardEvent) {
  if (!event.shiftKey) return false;
  if (event.key === "F7") {
    event.preventDefault();
    applyTheme(store.config?.theme || "paper");
    void loadBackground();
    return true;
  }
  const index = ["F1", "F2", "F3", "F4", "F5", "F8"].indexOf(event.key);
  const look = index >= 0 ? devLooks[index] : undefined;
  if (!look) return false;
  event.preventDefault();
  store.backgroundUrl = "";
  store.backgroundPreset = look.bg;
  applyTheme(look.theme);
  return true;
}

function onDevKey(event: KeyboardEvent) {
  if (!import.meta.env.DEV) return;
  if (applyDevLook(event)) return;
  if (event.ctrlKey && event.key === "F8") {
    event.preventDefault();
    seedDemoLog();
    store.logPanel = true;
    return;
  }
  if (event.ctrlKey && event.key === "F4") {
    event.preventDefault();
    store.page = "multiplayer";
    // 页面过渡结束后再切到陶瓦房间，两个时间点兜底组件挂载时机
    const push = () =>
      window.dispatchEvent(new CustomEvent("docl-dev-mp-tab", { detail: "room" }));
    window.setTimeout(push, 420);
    window.setTimeout(push, 900);
    return;
  }
  if (event.ctrlKey && event.key === "F3") {
    event.preventDefault();
    store.page = "download";
    // 社区资源默认停在原版列表，截图要的是整合包那一栏
    const push = () =>
      window.dispatchEvent(new CustomEvent("docl-dev-dl-category", { detail: "modpacks" }));
    window.setTimeout(push, 420);
    window.setTimeout(push, 900);
    return;
  }
  if (event.key === "F9") {
    event.preventDefault();
    store.maskIds = true;
    return;
  }
  if (event.key === "F10") {
    event.preventDefault();
    store.maskIds = false;
    return;
  }
  if (event.key === "F8") {
    event.preventDefault();
    store.logPanel = !store.logPanel;
    return;
  }
  const index = ["F1", "F2", "F3", "F4", "F5", "F6", "F7"].indexOf(event.key);
  if (index >= 0) {
    event.preventDefault();
    store.page = devPages[index];
  }
}

onMounted(async () => {
  window.addEventListener("docl-open-error", (event) => {
    notify((event as CustomEvent<string>).detail, "err");
  });
  if (import.meta.env.DEV) window.addEventListener("keydown", onDevKey);
  await initApp();
  void checkStartupUpdate();
  void listenDrop();
});

onBeforeUnmount(() => {
  unlistenDrop?.();
  window.removeEventListener("keydown", onDevKey);
});

/// 拖入文件后交给后端识别分类，识别结果决定弹窗里怎么装。
async function listenDrop() {
  try {
    unlistenDrop = await getCurrentWebview().onDragDropEvent((event) => {
      const payload = event.payload;
      if (payload.type === "enter" || payload.type === "over") {
        dropActive.value = true;
        return;
      }
      dropActive.value = false;
      if (payload.type === "drop") void handleDrop(payload.paths);
    });
  } catch {
    // 拖放不可用时不影响其他功能
  }
}

async function handleDrop(paths: string[]) {
  if (!paths.length) return;
  if (store.installingId) {
    notify("已有任务正在进行，请稍后再导入", "err");
    return;
  }
  try {
    const items = await api.dropInspect(paths);
    if (!items.length) {
      notify("没有找到可导入的文件", "err");
      return;
    }
    dropItems.value = items;
  } catch (err) {
    notify(`识别文件失败：${String(err)}`, "err");
  }
}

/// 关掉 Java 提示并跳到设置页的 Java 分类
function goJavaSettings() {
  store.javaPrompt = null;
  store.settingsFocus = "java";
  store.page = "settings";
}

/// 用户明知版本不匹配仍要继续启动
function retryLaunchWithJava() {
  const retry = store.javaPrompt?.retry;
  store.javaPrompt = null;
  retry?.();
}

/// 按设置里的更新策略在启动时静默检查；失败不打扰用户。
async function checkStartupUpdate() {
  const config = store.config;
  if (!config || config.update_mode === "off" || !config.update_url.trim()) return;
  try {
    const info = await api.updateCheck(config.update_url);
    if (!info.available) return;
    if (config.update_mode === "auto") {
      const path = await api.updateDownload(info.url, info.latest);
      startupUpdate.value = { info, path };
      notify(`已下载新版本 v${info.latest}，可在 设置 → 更新 中安装`);
    } else {
      startupUpdate.value = { info, path: null };
      notify(`发现新版本 v${info.latest}，可在 设置 → 更新 中查看`);
    }
  } catch {
    // 静默检查失败交给设置页手动重试
  }
}

const appStyle = computed(
  () =>
    ({
      "--bg-blur": `${store.config?.background_blur ?? 4}px`,
      "--glass-blur": `${store.config?.glass_blur ?? 18}px`,
      "--glass-alpha": String((store.config?.glass_opacity ?? 62) / 100),
      "--bg-dim": String((store.config?.background_dim ?? 0) / 100),
    }) as Record<string, string>
);

/// 背景适应方式：cover 铺满裁剪 / contain 完整显示 / stretch 拉伸 / center 原始大小
const bgFitStyle = computed<CSSProperties>(() => {
  switch (store.config?.background_fit) {
    case "contain":
      return { objectFit: "contain", transform: "none" };
    case "stretch":
      return { objectFit: "fill", transform: "none" };
    case "center":
      return { objectFit: "none", transform: "none" };
    default:
      return { objectFit: "cover" };
  }
});

/// 毛玻璃模式下让页面背景透出，配合系统 Acrylic 看到桌面
watch(
  () => store.config?.window_vibrancy,
  (value) => document.documentElement.classList.toggle("vibrancy", !!value),
  { immediate: true }
);

const views = {
  home: markRaw(HomeView),
  versions: markRaw(VersionsView),
  download: markRaw(DownloadView),
  accounts: markRaw(AccountsView),
  skins: markRaw(SkinsView),
  instance: markRaw(InstanceView),
  multiplayer: markRaw(MultiplayerView),
  settings: markRaw(SettingsView),
};

const currentView = computed(() => views[store.page as Page]);
</script>

<template>
  <div
    class="app glass"
    :class="{ 'no-anim': store.config && !store.config.animations }"
    :data-radius="store.config?.ui_radius ?? 'default'"
    :data-density="store.config?.ui_density ?? 'normal'"
    :style="appStyle"
  >
    <div
      v-if="store.backgroundUrl || store.backgroundPreset"
      class="wallpaper"
      :class="store.backgroundPreset ? `wallpaper--${store.backgroundPreset}` : ''"
      aria-hidden="true"
    >
      <img
        v-if="store.backgroundUrl"
        :src="store.backgroundUrl"
        :style="bgFitStyle"
        alt=""
      />
      <span v-else class="wallpaper__builtin" />
      <span class="wallpaper__tint" />
      <span class="wallpaper__dim" />
    </div>

    <div class="ambient" aria-hidden="true">
      <span class="ambient__orb ambient__orb--a" />
      <span class="ambient__orb ambient__orb--b" />
      <span class="ambient__grain" />
    </div>

    <TitleBar />

    <main class="main" :class="{ 'main--flush': store.page === 'home' }">
      <Transition name="page" mode="out-in">
        <component :is="currentView" :key="store.page" />
      </Transition>
    </main>

    <DownloadCenter />
    <LogPanel />
    <CrashDialog />

    <WoodModal
      v-if="store.javaPrompt"
      :title="store.javaPrompt.kind === 'missing' ? '缺少 Java 运行时' : 'Java 版本不匹配'"
      :width="440"
      :z-index="120"
      @close="store.javaPrompt = null"
    >
      <p class="java-prompt__text">{{ store.javaPrompt.message }}</p>
      <p class="java-prompt__hint muted small">
        <template v-if="store.javaPrompt.kind === 'missing'">
          可以到「设置 → Java 运行时」选择本机已有的 Java，或让启动器下载一个。
        </template>
        <template v-else>
          建议到「设置 → Java 运行时」把 Java 路径改为自动，或改选匹配的版本。
        </template>
      </p>
      <template #footer>
        <WoodButton variant="ghost" size="sm" @click="store.javaPrompt = null">取消</WoodButton>
        <WoodButton
          v-if="store.javaPrompt.retry"
          variant="soft"
          size="sm"
          @click="retryLaunchWithJava()"
        >
          仍然启动
        </WoodButton>
        <WoodButton variant="primary" size="sm" @click="goJavaSettings()">
          前往设置
        </WoodButton>
      </template>
    </WoodModal>

    <Transition name="drop">
      <div v-if="dropActive" class="drop-hint">
        <div class="drop-hint__card">
          <AppIcon name="package" :size="30" />
          <strong>松开以导入</strong>
          <span>自动识别模组、资源包、光影、数据包、存档与整合包</span>
        </div>
      </div>
    </Transition>

    <DropImportDialog
      v-if="dropItems.length"
      :items="dropItems"
      @close="dropItems = []"
      @done="dropItems = []"
    />

    <Transition name="toast">
      <div v-if="store.toast" class="toast" :class="{ 'toast--err': store.toast.kind === 'err' }">
        {{ store.toast.text }}
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.java-prompt__text {
  margin: 0 0 8px;
  font-size: 13px;
  line-height: 1.7;
  color: var(--ink);
}

.java-prompt__hint {
  margin: 0;
  line-height: 1.7;
}

.drop-hint {
  position: fixed;
  inset: 0;
  z-index: 78;
  display: grid;
  place-items: center;
  background: rgba(52, 38, 24, 0.26);
  backdrop-filter: blur(4px);
  pointer-events: none;
}

.drop-hint__card {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  padding: 30px 46px;
  border-radius: 18px;
  border: 2px dashed var(--moss);
  background: var(--panel);
  box-shadow: 0 24px 60px rgba(50, 34, 18, 0.28);
  color: var(--walnut-deep);
}

.drop-hint__card strong {
  font-size: 17px;
  letter-spacing: 0.04em;
}

.drop-hint__card span {
  font-size: 12px;
  color: var(--muted);
}

.drop-enter-active,
.drop-leave-active {
  transition: opacity 140ms ease;
}

.drop-enter-from,
.drop-leave-to {
  opacity: 0;
}

.toast-enter-active,
.toast-leave-active {
  transition: opacity 160ms ease;
}

.toast-enter-from,
.toast-leave-to {
  opacity: 0;
}
</style>
