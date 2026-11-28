<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { confirm } from "@tauri-apps/plugin-dialog";
import {
  api,
  openExternal,
  openFolder,
  type HallRoom,
  type LanView,
  type ServerEntry,
  type ServerPingResult,
  type ServerSnapshot,
  type ServerVersionOption,
  type TcState,
  type TcStatus,
} from "../api";
import AppIcon from "../components/AppIcon.vue";
import IconButton from "../components/IconButton.vue";
import WoodButton from "../components/WoodButton.vue";
import WoodInput from "../components/WoodInput.vue";
import WoodModal from "../components/WoodModal.vue";
import WoodSelect from "../components/WoodSelect.vue";
import WoodToggle from "../components/WoodToggle.vue";
import {
  forgetServer,
  launchServer,
  notify,
  rememberServer,
  selectedAccount,
  selectedVersion,
  store,
} from "../store";
import { kindLabel, skipWhenHidden } from "../utils";

type Tab = "hall" | "lan" | "room" | "direct" | "recent" | "server";

const tab = ref<Tab>("lan");
const lan = ref<LanView>({ rooms: [], error: null });
const hall = ref<HallRoom[]>([]);
const hallLoading = ref(false);
const hallError = ref("");
const scanning = ref(true);
const joining = ref("");
const directAddress = ref("");
const versionId = ref(selectedVersion.value?.id ?? "");
const now = ref(Math.floor(Date.now() / 1000));
let timer: number | undefined;
let serverTimer: number | undefined;
let pingTimer: number | undefined;

type PingState =
  | { status: "loading" }
  | { status: "ok"; result: ServerPingResult }
  | { status: "err"; message: string };

const pings = reactive<Record<string, PingState>>({});
const pinging = ref(false);
const favoriteAddress = ref("");

const serverVersions = ref<ServerVersionOption[]>([]);
const serverVersionId = ref("");
const serverStatus = ref<ServerSnapshot | null>(null);
const serverBusy = ref(false);
const consoleInput = ref("");
const consoleEl = ref<HTMLElement | null>(null);
const serverForm = reactive({
  port: 25565,
  motd: "DOCL 本地服务器",
  maxPlayers: 10,
  viewDistance: 8,
  // 离线账号无法通过正版验证，默认关闭；界面会同时给出公网风险提示
  onlineMode: selectedAccount.value?.kind !== "offline",
  whiteList: false,
  eula: false,
  playerName: selectedAccount.value?.name ?? "",
});
/// 控制台输入的本地预检结果：jvm / cjk / unknown，只在输入有问题时显示提示
const consoleHint = ref<"" | "jvm" | "cjk" | "unknown">("");

// 改动输入后收起上一次提示，避免旧提示一直挂着
watch(consoleInput, () => {
  if (consoleHint.value) consoleHint.value = "";
});
const reportOpen = ref(false);
const reportText = ref("");
const reportBusy = ref(false);
/// 当前登录的是离线账号：开正版验证会被服务端拒绝
const accountIsOffline = computed(() => selectedAccount.value?.kind === "offline");

watch(
  selectedAccount,
  (account) => {
    if (account && !serverForm.playerName) serverForm.playerName = account.name;
  },
  { immediate: true }
);

/// 会接收中文参数的聊天类指令；其它指令带中文基本是误把说明一起粘进来
const CHAT_COMMANDS = new Set([
  "say",
  "tell",
  "msg",
  "w",
  "me",
  "title",
  "tellraw",
  "teammsg",
  "broadcast",
]);

function consoleIssue(line: string): "" | "jvm" | "cjk" | "unknown" {
  if (/^-(?:[xX]|XX:|D)/.test(line)) return "jvm";
  if (/[^\x00-\x7F]/.test(line)) {
    const first = line.replace(/^\//, "").split(/\s+/)[0]?.toLowerCase() ?? "";
    if (!CHAT_COMMANDS.has(first)) return "cjk";
  }
  if (serverStatus.value?.loader === "vanilla" && /^spark\b/i.test(line)) return "unknown";
  return "";
}


const tc = ref<TcStatus | null>(null);
const tcPreparing = ref(false);
const tcError = ref("");
const tcBusy = ref(false);
const tcPlayer = ref(selectedAccount.value?.name ?? "");
const tcRoomInput = ref("");
let tcTimer: number | undefined;
/// 轮询开关：采用「上次请求结束再排下一次」，避免组件卡住时请求堆叠
let tcPolling = false;
/// 连续读取失败次数，达到阈值才把界面切成错误页，避免瞬时抖动打断房间
let tcFailures = 0;
/// 正在进行中的准备任务，重复调用直接复用
let tcPrepareTask: Promise<void> | null = null;

const versionOptions = computed(() => [
  ...store.installed.map((item) => {
    const meta = store.metas[item.id];
    const name = meta?.display_name?.trim() || item.id;
    return { value: item.id, label: `${name} · ${kindLabel(item.kind)}` };
  }),
]);

const recentServers = computed(() => store.config?.recent_servers ?? []);

const currentVersionLabel = computed(() => {
  const item = store.installed.find((v) => v.id === versionId.value);
  if (!item) return "未选择版本";
  const meta = store.metas[item.id];
  return meta?.display_name?.trim() || item.id;
});

const loaderLabels: Record<string, string> = {
  vanilla: "原版",
  fabric: "Fabric",
  forge: "Forge",
  neoforge: "NeoForge",
  quilt: "Quilt",
  paper: "Paper",
};

const serverVersionOptions = computed(() =>
  serverVersions.value.map((item) => ({
    value: item.id,
    label: `${item.name ? `${item.name} · ` : ""}${item.game_version || "未知版本"} · ${
      loaderLabels[item.loader] ?? item.loader
    } · Java ${item.java_major}`,
  }))
);

const selectedServerLoader = computed(
  () => serverVersions.value.find((item) => item.id === serverVersionId.value)?.loader ?? "vanilla"
);

/// 服务端整合包显示导入时记录的名称，普通版本直接显示版本 ID
function serverVersionLabel(id: string) {
  return serverVersions.value.find((item) => item.id === id)?.name || id;
}

/// 下拉里选中的是导入的服务端整合包（而不是已安装的游戏版本）
const selectedServerIsPack = computed(() => serverVersionId.value.startsWith("pack:"));

/// 删除导入的服务端包：解压目录和导入记录一起清掉
async function removeServerPack() {
  if (!selectedServerIsPack.value) return;
  if (serverBusy.value) {
    notify("服务端正在准备中，请稍后再删除", "err");
    return;
  }
  if (serverStatus.value?.version_id === serverVersionId.value) {
    notify("服务端正在运行，请先停止再删除", "err");
    return;
  }
  const label = serverVersionLabel(serverVersionId.value);
  const ok = await confirm(
    `将删除服务端包「${label}」的解压目录和导入记录。\n服务端存档、配置也会一并删除，无法恢复。`,
    { title: "删除服务端包", kind: "warning" }
  );
  if (!ok) return;
  try {
    await api.serverPackRemove(serverVersionId.value);
    notify("已删除服务端包");
    await loadServerVersions();
  } catch (err) {
    notify(`删除失败：${String(err)}`, "err");
  }
}

const serverUptime = computed(() => {
  if (!serverStatus.value) return "";
  const seconds = Math.max(0, now.value - serverStatus.value.started_at);
  const minutes = Math.floor(seconds / 60);
  const hours = Math.floor(minutes / 60);
  if (hours > 0) return `${hours} 小时 ${minutes % 60} 分`;
  if (minutes > 0) return `${minutes} 分 ${seconds % 60} 秒`;
  return `${seconds} 秒`;
});

/// 组件下载进度：只有后端真的在下载（有总字节数）时才展示百分比
const tcDownload = computed(() => {
  const p = store.progress;
  if (!p || p.stage !== "terracotta" || !p.bytes_total) return null;
  return {
    percent: Math.min(100, Math.round((p.bytes_done / p.bytes_total) * 100)),
    done: (p.bytes_done / 1024 / 1024).toFixed(1),
    total: (p.bytes_total / 1024 / 1024).toFixed(1),
  };
});

const tcStateName = computed(() => tc.value?.state?.state ?? "");
const tcWaiting = computed(
  () => !tc.value?.state || tcStateName.value === "waiting"
);
const tcRoom = computed(() => tc.value?.state?.room ?? "");
const tcUrl = computed(() => tc.value?.state?.url ?? "");

/// 陶瓦会一直扫描直到发现局域网世界，这里给它加个上限，避免界面无限等待
const SCAN_TIMEOUT = 45;
const scanSeconds = ref(0);
let scanTimer: number | undefined;

function startScanClock() {
  if (scanTimer) return;
  scanSeconds.value = 0;
  scanTimer = window.setInterval(() => {
    scanSeconds.value += 1;
    if (scanSeconds.value >= SCAN_TIMEOUT) void cancelScan(true);
  }, 1000);
}

function stopScanClock() {
  if (scanTimer) {
    window.clearInterval(scanTimer);
    scanTimer = undefined;
  }
  scanSeconds.value = 0;
}

async function cancelScan(timedOut: boolean) {
  stopScanClock();
  try {
    await api.terracottaReset();
  } catch {
    // 重置失败也让界面回到可操作状态
  }
  await refreshTc();
  if (timedOut) {
    tcError.value =
      "超过 45 秒没有发现局域网世界。请先在游戏里进入存档，按 Esc 点「对局域网开放」，再重新创建房间。";
  }
}

watch(tcStateName, (state) => {
  // 倒计时只跟房间页绑定，离开页面时不继续在后台取消扫描
  if (tab.value !== "room") return;
  if (state === "host-scanning") startScanClock();
  else stopScanClock();
});

const tcProfiles = computed(() => tc.value?.state?.profiles ?? []);
const tcExceptionText = computed(() => {
  switch (tc.value?.state?.type ?? 0) {
    case 1:
      return "与房主的连接中断";
    case 2:
      return "加入方 EasyTier 崩溃";
    case 3:
      return "房主 EasyTier 崩溃";
    case 4:
      return "房主的 Minecraft 服务器已失联（可能退出了存档）";
    case 5:
      return "联机协议异常，请稍后重试";
    default:
      return "无法连接房主，房间可能已关闭或网络不稳定";
  }
});
const tcDifficulty = computed(() => {
  switch (tc.value?.state?.difficulty) {
    case "EASIEST":
      return "连接难度：极低";
    case "SIMPLE":
      return "连接难度：简单";
    case "TOUGH":
      return "连接难度：较高";
    case "MEDIUM":
      return "连接难度：中等";
    default:
      return "正在评估连接难度…";
  }
});

const tcTough = computed(() => tc.value?.state?.difficulty === "TOUGH");
/// 断线重试用的房间码：异常状态里可能还留着，页面输入框兜底
const tcRetryRoom = computed(() => (tc.value?.state?.room || tcRoomInput.value).trim());

/// 本地「最近一起玩的人」：陶瓦没有好友接口，只能自己记
interface TcRecent {
  machine_id: string;
  name: string;
  room: string;
  ts: number;
}

const RECENTS_KEY = "docl.tc.recents";
const recents = ref<TcRecent[]>([]);
/// 上一次看到的房间成员，用来提示有人进入/离开
let knownMembers: { id: string; name: string }[] = [];

function loadRecents() {
  try {
    const raw = localStorage.getItem(RECENTS_KEY);
    const parsed = raw ? JSON.parse(raw) : [];
    recents.value = Array.isArray(parsed) ? parsed : [];
  } catch {
    recents.value = [];
  }
}

function saveRecents() {
  try {
    localStorage.setItem(RECENTS_KEY, JSON.stringify(recents.value));
  } catch {
    // 隐私模式写不进去就算了，不影响联机
  }
}

function clearRecents() {
  recents.value = [];
  saveRecents();
}

/// 房主视角自己是 HOST，房客视角自己是 LOCAL，其余才是「一起玩过的人」
function rememberPlayers(state: TcState) {
  const room = (state.room ?? "").trim();
  if (!room) return;
  const selfKind = state.state === "guest-ok" ? "LOCAL" : "HOST";
  const ts = Date.now();
  let changed = false;
  for (const profile of state.profiles ?? []) {
    if (!profile.machine_id) continue;
    if (profile.kind === selfKind || profile.kind === "LOCAL") continue;
    const entry: TcRecent = {
      machine_id: profile.machine_id,
      name: profile.name || "未知玩家",
      room,
      ts,
    };
    const index = recents.value.findIndex((item) => item.machine_id === profile.machine_id);
    if (index >= 0) {
      // 状态会持续轮询，只有名字或房间变化时才写 localStorage
      const prev = recents.value[index];
      if (prev.name !== entry.name || prev.room !== entry.room) {
        recents.value[index] = entry;
        changed = true;
      }
    } else {
      recents.value.unshift(entry);
      changed = true;
    }
  }
  if (changed) {
    recents.value = recents.value.slice(0, 12);
    saveRecents();
  }
}

/// 写入新状态，顺带处理成员变动提示与本地记录
function applyTcState(next: TcStatus) {
  const state = next.state;
  tc.value = next;
  if (!state) return;
  const active = state.state === "host-ok" || state.state === "guest-ok";
  if (!active) {
    knownMembers = [];
    return;
  }
  const now = (state.profiles ?? [])
    .filter((profile) => profile.kind !== "LOCAL")
    .map((profile) => ({
      id: profile.machine_id,
      name: profile.name || "有人",
    }));
  // 第一次拿到成员列表时只记录不提示，避免刚进房就刷一堆通知
  if (knownMembers.length) {
    for (const member of now) {
      if (!knownMembers.some((item) => item.id === member.id)) {
        notify(`${member.name} 加入了房间`);
      }
    }
    for (const member of knownMembers) {
      if (!now.some((item) => item.id === member.id)) {
        notify(`${member.name} 离开了房间`);
      }
    }
  }
  knownMembers = now;
  rememberPlayers(state);
}

watch(
  () => store.installed,
  (list) => {
    if (!versionId.value || !list.some((item) => item.id === versionId.value)) {
      versionId.value = selectedVersion.value?.id ?? list[0]?.id ?? "";
    }
  },
  { immediate: true }
);

function secondsLabel(lastSeen: number) {
  const diff = Math.max(0, now.value - lastSeen);
  if (diff < 3) return "刚刚";
  if (diff < 60) return `${diff} 秒前`;
  return `${Math.floor(diff / 60)} 分钟前`;
}

function usedLabel(lastUsed: number) {
  if (!lastUsed) return "";
  const date = new Date(lastUsed * 1000);
  return `${String(date.getMonth() + 1).padStart(2, "0")}-${String(
    date.getDate()
  ).padStart(2, "0")} ${String(date.getHours()).padStart(2, "0")}:${String(
    date.getMinutes()
  ).padStart(2, "0")}`;
}

async function refreshLan() {
  try {
    lan.value = await api.lanRooms();
  } catch (err) {
    lan.value = { rooms: [], error: String(err) };
  } finally {
    scanning.value = false;
    now.value = Math.floor(Date.now() / 1000);
  }
}

async function refreshHall() {
  if (hallLoading.value) return;
  hallLoading.value = true;
  hallError.value = "";
  try {
    hall.value = await api.hallRooms();
  } catch (err) {
    hall.value = [];
    hallError.value = String(err);
  } finally {
    hallLoading.value = false;
  }
}

async function joinHallRoom(room: HallRoom) {
  // 进房前校验版本 / 加载器 / Mod 列表；大厅源没广播这些字段时会自动跳过
  if (versionId.value) {
    try {
      const check = await api.roomModCheck(
        versionId.value,
        room.version ?? "",
        room.loader ?? "",
        room.mods ?? []
      );
      const lines = [...check.problems];
      if (check.missing.length) {
        lines.push(`缺少房间需要的 Mod：${check.missing.slice(0, 8).join("、")}`);
      }
      if (lines.length) {
        const proceed = await confirm(`${lines.join("\n")}\n\n仍要加入这个房间吗？`, {
          title: "房间内容与当前实例不一致",
          kind: "warning",
        });
        if (!proceed) {
          tab.value = "room";
          return;
        }
      }
    } catch {
      // 校验失败不阻塞进房，交给陶瓦的报错提示
    }
  }
  tcRoomInput.value = room.room;
  tab.value = "room";
  // 首次使用需要下载组件，这里等准备流程真正结束再判断
  if (!tc.value?.running && !tcError.value) await prepareTc();
  if (!tc.value?.running) return;
  await joinTc();
}

async function join(address: string) {
  if (joining.value) return;
  if (!versionId.value) {
    notify("请先安装并选择一个游戏版本", "err");
    store.page = "download";
    return;
  }
  joining.value = address;
  try {
    await rememberServer(address);
    await launchServer(address, versionId.value);
  } finally {
    joining.value = "";
  }
}

function joinDirect() {
  const value = directAddress.value.trim();
  if (!value) {
    notify("请输入服务器地址", "err");
    return;
  }
  void join(value);
}

function removeRecent(address: string) {
  void forgetServer(address);
  delete pings[address];
}

async function pingAll() {
  if (pinging.value) return;
  const targets = recentServers.value.map((item) => item.address);
  if (!targets.length) return;
  pinging.value = true;
  for (const address of targets) pings[address] = { status: "loading" };
  await Promise.all(
    targets.map(async (address) => {
      try {
        const result = await api.serverPing(address);
        pings[address] = { status: "ok", result };
      } catch (err) {
        pings[address] = { status: "err", message: String(err) };
      }
    })
  );
  pinging.value = false;
}

async function addFavorite() {
  const value = favoriteAddress.value.trim();
  if (!value) {
    notify("请输入服务器地址", "err");
    return;
  }
  await rememberServer(value);
  favoriteAddress.value = "";
  void pingAll();
}

function faviconOf(address: string): string {
  const state = pings[address];
  return state?.status === "ok" ? state.result.favicon ?? "" : "";
}

function titleOf(server: ServerEntry): string {
  const state = pings[server.address];
  if (state?.status === "ok" && state.result.motd) return state.result.motd;
  return server.address;
}

function stateLine(server: ServerEntry): string {
  const state = pings[server.address];
  if (!state || state.status === "loading") return `${server.address} · 查询中…`;
  if (state.status === "err") return `${server.address} · 离线或无法连接`;
  const info = state.result;
  const parts = [
    server.address,
    `${info.players_online}/${info.players_max} 人`,
    `${info.latency_ms}ms`,
  ];
  if (info.version_name) parts.push(info.version_name);
  return parts.join(" · ");
}

function iconClass(address: string): string {
  const state = pings[address];
  if (!state || state.status === "loading") return "";
  return state.status === "ok" ? "mp-room__icon--on" : "mp-room__icon--off";
}

function tipOf(address: string): string {
  const state = pings[address];
  if (!state) return "";
  if (state.status === "loading") return "正在查询服务器状态…";
  if (state.status === "err") return state.message;
  return state.result.players_sample.length
    ? `在线玩家：${state.result.players_sample.join("、")}`
    : "服务器在线";
}

function startPingPolling() {
  stopPingPolling();
  pingTimer = window.setInterval(skipWhenHidden(() => void pingAll()), 30000);
}

function stopPingPolling() {
  if (pingTimer) {
    window.clearInterval(pingTimer);
    pingTimer = undefined;
  }
}

async function loadServerVersions() {
  try {
    serverVersions.value = await api.serverVersions();
    if (
      !serverVersionId.value ||
      !serverVersions.value.some((item) => item.id === serverVersionId.value)
    ) {
      serverVersionId.value = serverVersions.value[0]?.id ?? "";
    }
  } catch {
    serverVersions.value = [];
  }
}

async function refreshServer() {
  try {
    serverStatus.value = await api.serverStatus();
    now.value = Math.floor(Date.now() / 1000);
  } catch {
    serverStatus.value = null;
  }
}

async function startServer() {
  if (serverBusy.value) return;
  if (!serverForm.eula) {
    notify("请先勾选同意 Minecraft EULA", "err");
    return;
  }
  if (!serverVersionId.value) {
    notify("没有可用于开服的版本", "err");
    return;
  }
  if (serverForm.whiteList && !serverForm.playerName.trim()) {
    // 不硬拦：允许先开服再在控制台 whitelist add，但必须让用户知道现在谁都进不来
    const ok = await confirm(
      "已开启白名单但没填「你的游戏 ID」，开服后暂时没有人能进入。\n" +
        "可以开服后在控制台执行 whitelist add <名字>，或现在回去填上。是否继续开服？",
      { title: "白名单为空", kind: "warning" }
    );
    if (!ok) return;
  }
  serverBusy.value = true;
  try {
    serverStatus.value = await api.serverStart(serverVersionId.value, {
      port: Number(serverForm.port) || 25565,
      motd: serverForm.motd,
      max_players: Number(serverForm.maxPlayers) || 10,
      view_distance: Number(serverForm.viewDistance) || 8,
      online_mode: serverForm.onlineMode,
      white_list: serverForm.whiteList,
      accept_eula: true,
      player_name: serverForm.playerName.trim(),
    });
    notify("服务端已启动，正在等待就绪");
  } catch (err) {
    notify(`启动服务端失败：${String(err)}`, "err");
  } finally {
    serverBusy.value = false;
    await refreshServer();
  }
}

async function stopServer() {
  try {
    await api.serverStop();
    notify("已发送停止命令");
  } catch (err) {
    notify(`停止失败：${String(err)}`, "err");
  }
}

async function sendConsole() {
  const line = consoleInput.value.trim();
  if (!line) return;
  const issue = consoleIssue(line);
  consoleHint.value = issue;
  // JVM 参数和中文指令 100% 是误操作，直接拦截；其它只提示，用户装了插件仍可发送
  if (issue === "jvm") {
    notify("这是 Java 虚拟机参数，需要在设置里填写", "err");
    return;
  }
  if (issue === "cjk") {
    notify("控制台只接受英文服务端指令，含中文会被服务端拒绝", "err");
    return;
  }
  try {
    await api.serverCommand(line);
    consoleInput.value = "";
  } catch (err) {
    notify(`发送失败：${String(err)}`, "err");
  }
}

function openPerformanceSettings() {
  store.page = "settings";
  store.settingsFocus = "performance";
}

interface ConsoleNote {
  level: "info" | "warn" | "error";
  text: string;
  action?: { label: string; kind: "whitelist" | "settings"; name?: string };
}

/// 已知日志的注解：把 “警告原文” 翻译成 “发生了什么 + 该点哪里”
function consoleNote(line: string): ConsoleNote | null {
  if (/Can't keep up!.*Running \d+ms or \d+ ticks behind/i.test(line)) {
    return {
      level: "info",
      text: "启动或保存区块瞬间的阻塞，原版服务端正常现象，无需处理；若正好此刻登录，稍等重进即可。",
    };
  }
  if (/SERVER IS RUNNING IN OFFLINE\/INSECURE MODE/i.test(line)) {
    return {
      level: "info",
      text: "离线模式已生效。自己或局域网游玩没问题，开公网请改回正版验证。",
    };
  }
  if (/handleDisconnection\(\) called twice/i.test(line)) {
    return { level: "info", text: "无害提示，可忽略。" };
  }
  if (/lost connection: Disconnected/i.test(line)) {
    return {
      level: "warn",
      text: "客户端主动断开。若本机或朋友反复被踢，先检查：离线账号遇上正版验证（需停服后关闭），或客户端与服务端版本不一致。",
    };
  }
  if (/not white-listed on this server/i.test(line)) {
    const name =
      /Disconnecting\s+(\w+)/i.exec(line)?.[1] ??
      /^\[[^\]]+\]\s+(\w+)\s+\/\d/i.exec(line)?.[1];
    return {
      level: "error",
      text: "该账号不在白名单，服务器把连接拒绝了。",
      action: name
        ? { label: `加入白名单：${name}`, kind: "whitelist", name }
        : undefined,
    };
  }
  if (/Address already in use|Failed to bind to port|BindException/i.test(line)) {
    return {
      level: "error",
      text: "端口被占用：可能已有一个服务端在运行，换一个端口后重启即可。",
    };
  }
  if (/UnsupportedClassVersionError|class file version/i.test(line)) {
    return {
      level: "error",
      text: "Java 版本过低，加载器或游戏需要更高版本。",
      action: { label: "打开设置", kind: "settings" },
    };
  }
  if (/OutOfMemoryError|Java heap space/i.test(line)) {
    return {
      level: "error",
      text: "内存不足。原版服务端主要吃单线程 CPU，调内存收益有限，但过小会直接崩溃。",
      action: { label: "打开设置", kind: "settings" },
    };
  }
  if (/Unknown or incomplete command/i.test(line)) {
    return {
      level: "warn",
      text: "控制台只接受服务端指令；Java 参数请在「设置 → 性能与参数」里填写。",
    };
  }
  return null;
}

/// 日志行内容不变时注解也不会变；后端每 1.5 秒回传全量日志，
/// 逐行跑正则太浪费，这里按行缓存，只有新行才重新匹配。
const consoleNoteCache = new Map<string, ConsoleNote | null>();

function cachedNote(line: string): ConsoleNote | null {
  const key = `${serverStatus.value?.loader ?? ""}\u0000${line}`;
  const hit = consoleNoteCache.get(key);
  if (hit !== undefined) return hit;
  const note = consoleNote(line);
  // 日志最多 400 行，超出后整体清空，避免长时间运行时无限增长
  if (consoleNoteCache.size > 1200) consoleNoteCache.clear();
  consoleNoteCache.set(key, note);
  return note;
}

const consoleLines = computed(() =>
  (serverStatus.value?.log ?? []).map((line) => ({ line, note: cachedNote(line) }))
);

function runNoteAction(action: NonNullable<ConsoleNote["action"]>) {
  if (action.kind === "whitelist" && action.name) {
    void addWhitelist(action.name);
    return;
  }
  openPerformanceSettings();
}

async function addWhitelist(name: string) {
  try {
    await api.serverCommand(`whitelist add ${name}`);
    notify(`已发送：whitelist add ${name}`);
    setTimeout(() => void refreshServer(), 800);
  } catch (err) {
    notify(`加入白名单失败：${String(err)}`, "err");
  }
}

async function makeReport() {
  reportBusy.value = true;
  try {
    reportText.value = await api.serverDiagnostic();
    reportOpen.value = true;
  } catch (err) {
    notify(`生成诊断报告失败：${String(err)}`, "err");
  } finally {
    reportBusy.value = false;
  }
}

async function copyReport() {
  try {
    await navigator.clipboard.writeText(reportText.value);
    notify("诊断报告已复制，可直接发给客服或其它 AI");
  } catch (err) {
    notify(`复制失败，请手动全选复制：${String(err)}`, "err");
  }
}

function openServerDir() {
  if (serverStatus.value?.dir) openFolder(serverStatus.value.dir);
}

async function copyAddress(text: string) {
  try {
    await navigator.clipboard.writeText(text);
    notify("地址已复制");
  } catch (err) {
    notify(`复制失败：${String(err)}`, "err");
  }
}

function prepareTc(): Promise<void> {
  if (tcPrepareTask) return tcPrepareTask;
  tcPreparing.value = true;
  tcError.value = "";
  tcFailures = 0;
  tcPrepareTask = (async () => {
    try {
      tc.value = await api.terracottaPrepare();
      if (tc.value.running) {
        startTcPolling();
        if (tab.value === "room" && tcStateName.value === "host-scanning") startScanClock();
      }
    } catch (err) {
      tcError.value = String(err);
    } finally {
      tcPreparing.value = false;
      tcPrepareTask = null;
    }
  })();
  return tcPrepareTask;
}

/// 进入房间页时同步一次状态：已有实例（含其他启动器启动的）直接展示，
/// 不触发组件下载和进程启动；真正的启动要等用户点「启动陶瓦联机」。
async function syncTc() {
  // 正在准备组件时不要用旧状态覆盖进度
  if (tcPrepareTask) return;
  try {
    const next = await api.terracottaStatus();
    applyTcState(next);
    if (next.running) {
      startTcPolling();
      if (tab.value === "room" && tcStateName.value === "host-scanning") startScanClock();
    }
  } catch {
    // 读不到状态不打扰用户，点「启动陶瓦联机」时会再走一次准备流程
  }
}

async function refreshTc() {
  if (!tc.value?.running) return;
  try {
    const next = await api.terracottaStatus();
    if (next.error) {
      tcFailures += 1;
      if (tcFailures >= 3) applyTcState(next);
      return;
    }
    tcFailures = 0;
    applyTcState(next);
  } catch {
    tcFailures += 1;
    if (tcFailures >= 3 && tc.value) {
      tc.value = { ...tc.value, error: "无法读取陶瓦联机状态" };
    }
  }
}

function startTcPolling() {
  if (tcPolling) return;
  tcPolling = true;
  if (!tcTimer) tcTimer = window.setTimeout(() => void pollTc(), 0);
}

function stopTcPolling() {
  tcPolling = false;
  if (tcTimer) {
    window.clearTimeout(tcTimer);
    tcTimer = undefined;
  }
}

/// 连接 / 扫描阶段要快速反馈，稳定状态放慢，减少本地状态请求
function tcPollDelay() {
  switch (tcStateName.value) {
    case "guest-connecting":
    case "guest-starting":
    case "host-starting":
    case "host-scanning":
    case "exception":
      return 700;
    default:
      return 1800;
  }
}

/// 上一次请求结束后再排下一次：组件卡住时不会越堆越多
async function pollTc() {
  tcTimer = undefined;
  if (!tcPolling) return;
  if (!document.hidden) await refreshTc();
  if (tcPolling) tcTimer = window.setTimeout(() => void pollTc(), tcPollDelay());
}

async function hostTc() {
  if (tcBusy.value) return;
  tcBusy.value = true;
  tcError.value = "";
  try {
    await api.terracottaHost(null, tcPlayer.value.trim() || null);
    await refreshTc();
  } catch (err) {
    notify(`创建房间失败：${String(err)}`, "err");
  } finally {
    tcBusy.value = false;
  }
}

async function joinTc() {
  if (tcBusy.value) return;
  const room = tcRoomInput.value.trim();
  if (!room) {
    notify("请输入房间邀请码", "err");
    return;
  }
  tcBusy.value = true;
  try {
    await api.terracottaJoin(room, tcPlayer.value.trim() || null);
    await refreshTc();
  } catch (err) {
    notify(`加入房间失败：${String(err)}`, "err");
  } finally {
    tcBusy.value = false;
  }
}

async function leaveTc() {
  try {
    await api.terracottaReset();
    await refreshTc();
    tcRoomInput.value = "";
  } catch (err) {
    notify(`退出房间失败：${String(err)}`, "err");
  }
}

/// 断线后一键重试：先回到空闲状态，再用同一房间码重新加入
async function retryTc() {
  if (tcBusy.value) return;
  const room = tcRetryRoom.value;
  if (!room) return;
  tcBusy.value = true;
  tcError.value = "";
  try {
    await api.terracottaReset().catch(() => {});
    tcRoomInput.value = room;
    await api.terracottaJoin(room, tcPlayer.value.trim() || null);
  } catch (err) {
    notify(`重新加入失败：${String(err)}`, "err");
  } finally {
    tcBusy.value = false;
    await refreshTc();
  }
}

watch(
  () => serverStatus.value?.log.length,
  async () => {
    await nextTick();
    const el = consoleEl.value;
    if (el) el.scrollTop = el.scrollHeight;
  }
);

watch(tab, (value) => {
  if (value === "hall") {
    void refreshHall();
  }
  if (value === "server") {
    void loadServerVersions();
    void refreshServer();
  }
  if (value === "room") {
    void syncTc();
    // 回到房间页时如果还在扫描，重新开始倒计时（离开期间暂停）
    if (tcStateName.value === "host-scanning") startScanClock();
  } else {
    stopTcPolling();
    stopScanClock();
  }
  if (value === "recent") {
    void pingAll();
    startPingPolling();
  } else {
    stopPingPolling();
  }
});

/// 截图脚本用：开发模式下可直接切到陶瓦房间
function onDevMpTab(event: Event) {
  tab.value = (event as CustomEvent<string>).detail as typeof tab.value;
}

onMounted(() => {
  if (import.meta.env.DEV) window.addEventListener("docl-dev-mp-tab", onDevMpTab);
  loadRecents();
  void refreshLan();
  void loadServerVersions();
  void refreshServer();
  timer = window.setInterval(skipWhenHidden(() => void refreshLan()), 2500);
  serverTimer = window.setInterval(skipWhenHidden(() => void refreshServer()), 1500);
});

onBeforeUnmount(() => {
  window.removeEventListener("docl-dev-mp-tab", onDevMpTab);
  if (timer) window.clearInterval(timer);
  if (serverTimer) window.clearInterval(serverTimer);
  stopScanClock();
  stopTcPolling();
  stopPingPolling();
});
</script>

<template>
  <div class="page multi">
    <header class="page-head">
      <div class="page-head__actions">
        <span class="tag" :class="{ 'tag--grass': !!selectedAccount }">
          {{ selectedAccount ? `账号 ${selectedAccount.name}` : "未登录（离线）" }}
        </span>
        <span class="tag" :class="{ 'tag--grass': store.gameRunning }">
          {{ store.gameRunning ? "游戏运行中" : "空闲" }}
        </span>
      </div>
    </header>

    <div class="mp-layout">
      <aside class="mp-side card">
        <div class="mp-side__group">
          <span class="side-label">加入游戏</span>
          <button
            class="mp-nav"
            :class="{ 'mp-nav--active': tab === 'hall' }"
            @click="tab = 'hall'"
          >
            <AppIcon name="news" :size="16" />
            <span>房间大厅</span>
            <span v-if="hall.length" class="mp-nav__count mono">
              {{ hall.length }}
            </span>
          </button>
          <button
            class="mp-nav"
            :class="{ 'mp-nav--active': tab === 'lan' }"
            @click="tab = 'lan'"
          >
            <AppIcon name="globe" :size="16" />
            <span>局域网发现</span>
            <span v-if="lan.rooms.length" class="mp-nav__count mono">
              {{ lan.rooms.length }}
            </span>
          </button>
          <button
            class="mp-nav"
            :class="{ 'mp-nav--active': tab === 'room' }"
            @click="tab = 'room'"
          >
            <AppIcon name="layers" :size="16" />
            <span>陶瓦房间</span>
          </button>
          <button
            class="mp-nav"
            :class="{ 'mp-nav--active': tab === 'direct' }"
            @click="tab = 'direct'"
          >
            <AppIcon name="link" :size="16" />
            <span>直连地址</span>
          </button>
        </div>

        <div class="mp-side__group">
          <span class="side-label">记录</span>
          <button
            class="mp-nav"
            :class="{ 'mp-nav--active': tab === 'recent' }"
            @click="tab = 'recent'"
          >
            <AppIcon name="clock" :size="16" />
            <span>最近服务器</span>
            <span v-if="recentServers.length" class="mp-nav__count mono">
              {{ recentServers.length }}
            </span>
          </button>
        </div>

        <div class="mp-side__group">
          <span class="side-label">创建</span>
          <button
            class="mp-nav"
            :class="{ 'mp-nav--active': tab === 'server' }"
            @click="tab = 'server'"
          >
            <AppIcon name="database" :size="16" />
            <span>本地开服</span>
          </button>
        </div>
      </aside>

      <div class="mp-content">
        <section v-if="tab !== 'server'" class="mp-context card reveal">
          <div class="mp-context__text">
            <strong>加入时使用的版本</strong>
          </div>
          <div class="mp-bar__right">
            <WoodSelect
              v-if="versionOptions.length"
              v-model="versionId"
              :options="versionOptions"
            />
            <WoodButton
              v-else
              variant="primary"
              size="sm"
              @click="store.page = 'download'"
            >
              <AppIcon name="download" :size="14" />
              去下载版本
            </WoodButton>
          </div>
        </section>

    <section v-if="tab === 'hall'" class="card card-pad mp-panel reveal">
      <div class="mp-panel__head">
        <div>
          <div class="section-title">房间大厅</div>
        </div>
        <IconButton
          icon="refresh"
          label="刷新大厅"
          :size="32"
          :disabled="hallLoading"
          @click="refreshHall"
        />
      </div>

      <p class="muted small mp-hint">
        大厅列表来自设置里的目录地址；陶瓦本身只支持邀请码，需要自建或托管一个返回 JSON 的房间服务。
      </p>

      <div v-if="!store.config?.hall_url" class="mp-empty mp-empty--card">
        <span class="mp-empty__icon">
          <AppIcon name="news" :size="20" />
        </span>
        <div class="mp-empty__text">
          <strong>还没有配置房间大厅</strong>
          <span class="muted small">
            在「设置 → 联机」里填写大厅地址后，这里会列出公开房间。
          </span>
        </div>
        <WoodButton variant="soft" size="sm" @click="store.page = 'settings'">
          去设置
        </WoodButton>
      </div>

      <div v-else-if="hallLoading" class="mp-empty">
        <span class="mp-pulse" />
        正在读取大厅…
      </div>
      <div v-else-if="hallError" class="mp-alert">
        <AppIcon name="log" :size="14" />
        {{ hallError }}
        <WoodButton variant="ghost" size="sm" @click="refreshHall">重试</WoodButton>
      </div>
      <div v-else-if="!hall.length" class="state state--inline">
        <span class="state__icon"><AppIcon name="globe" :size="18" /></span>
        <div class="state__text">
          <p class="state__title">大厅暂时没有房间</p>
          <p class="state__hint">稍后再刷新，或自己开一个房间等朋友进来。</p>
        </div>
      </div>
      <div v-else class="mp-list">
        <div v-for="room in hall" :key="room.room" class="mp-room">
          <span class="mp-room__icon">
            <AppIcon name="globe" :size="18" />
          </span>
          <span class="mp-room__text">
            <strong>{{ room.name || room.room }}</strong>
            <span>
              {{ room.motd || `邀请码 ${room.room}` }}
              <template v-if="room.version"> · {{ room.version }}</template>
              <template v-if="room.mods?.length"> · {{ room.mods.length }} 个 Mod</template>
              <template v-if="room.players || room.max_players">
                · {{ room.players }}/{{ room.max_players || "?" }}
              </template>
            </span>
          </span>
          <span class="mp-room__time mono">
            {{ room.updated ? secondsLabel(room.updated) : "" }}
          </span>
          <WoodButton
            variant="primary"
            size="sm"
            :disabled="tcBusy"
            @click="joinHallRoom(room)"
          >
            <AppIcon name="play" :size="12" />
            加入
          </WoodButton>
        </div>
      </div>
    </section>

    <section v-else-if="tab === 'lan'" class="card card-pad mp-panel reveal">
      <div class="mp-panel__head">
        <div>
          <div class="section-title">局域网世界</div>
        </div>
        <IconButton icon="refresh" label="刷新房间" :size="32" @click="refreshLan" />
      </div>

      <p class="muted small mp-hint">
        在游戏里对局域网开放后，同一网络下的世界会自动出现在这里；穿透联机后续接入。
      </p>

      <div v-if="lan.error" class="mp-alert">
        <AppIcon name="log" :size="14" />
        {{ lan.error }}
      </div>

      <div v-if="scanning" class="mp-empty">
        <span class="mp-pulse" />
        正在扫描局域网世界…
      </div>
      <div v-else-if="!lan.rooms.length" class="mp-empty mp-empty--card">
        <span class="mp-empty__icon">
          <AppIcon name="globe" :size="20" />
        </span>
        <div class="mp-empty__text">
          <strong>还没有发现局域网世界</strong>
          <span class="muted small">
            在游戏里进入存档 → 按 Esc → 点「对局域网开放」，同一网络下的世界会很快出现在这里。
          </span>
        </div>
        <WoodButton variant="ghost" size="sm" @click="refreshLan">重新扫描</WoodButton>
      </div>
      <div v-else class="mp-list">
        <div v-for="room in lan.rooms" :key="room.id" class="mp-room">
          <span class="mp-room__icon">
            <AppIcon name="globe" :size="18" />
          </span>
          <span class="mp-room__text">
            <strong>{{ room.motd }}</strong>
            <span class="mono">{{ room.address }}</span>
          </span>
          <span class="mp-room__time mono">{{ secondsLabel(room.last_seen) }}</span>
          <WoodButton
            variant="primary"
            size="sm"
            :disabled="!!joining"
            @click="join(room.address)"
          >
            <AppIcon name="play" :size="12" />
            {{ joining === room.address ? "连接中…" : "加入" }}
          </WoodButton>
        </div>
      </div>
    </section>

    <section v-else-if="tab === 'room'" class="tc-panel card reveal">
      <header class="tc-head">
        <span class="tc-head__title">陶瓦房间</span>
        <div class="tc-head__state">
          <span class="tc-dot" :class="{ 'tc-dot--on': tc?.running }" />
          <span class="mono">{{ tc?.running ? "RUNNING" : "IDLE" }}</span>
          <span v-if="tc?.running" class="tc-version mono">{{ tc.version || "联机" }}</span>
        </div>
      </header>

      <p class="tc-desc">
        基于 EasyTier 的 P2P 联机，与 HMCL / PCL-CE 等启动器互通。房主先在存档里「对局域网开放」，房客输入邀请码即可。
      </p>

      <div v-if="tcPreparing" class="tc-state">
        <span class="mp-pulse" />
        <template v-if="tcDownload">
          正在下载陶瓦联机组件 {{ tcDownload.percent }}%（{{ tcDownload.done }} /
          {{ tcDownload.total }} MB）
        </template>
        <template v-else>
          正在准备陶瓦联机组件，首次使用会下载约 10 MB…
        </template>
      </div>

      <div v-else-if="tcError" class="tc-alert">
        <span class="tc-alert__text">{{ tcError }}</span>
        <WoodButton variant="ghost" size="sm" @click="prepareTc">重试</WoodButton>
      </div>

      <div v-else-if="tc?.error" class="tc-alert">
        <span class="tc-alert__text">{{ tc.error }}</span>
        <WoodButton variant="ghost" size="sm" @click="prepareTc">重试</WoodButton>
      </div>

      <template v-else-if="tc?.running">
        <template v-if="tcWaiting">
          <div class="tc-form">
            <label class="tc-field">
              <span class="tc-field__label">玩家显示名</span>
              <WoodInput v-model="tcPlayer" />
            </label>
            <label class="tc-field">
              <span class="tc-field__label">房间邀请码（加入时填写）</span>
              <WoodInput v-model="tcRoomInput" mono placeholder="U/XXXX-XXXX-XXXX-XXXX" />
            </label>
          </div>
          <div class="tc-actions">
            <WoodButton variant="primary" size="md" :disabled="tcBusy" @click="hostTc">
              创建房间
            </WoodButton>
            <WoodButton variant="ghost" size="md" :disabled="tcBusy" @click="joinTc">
              加入房间
            </WoodButton>
          </div>
          <p class="tc-hint">创建房间前，请先在游戏里进入存档并点「对局域网开放」；加入房间只需邀请码。</p>
        </template>

        <div v-else-if="tcStateName === 'host-scanning'" class="tc-scan">
          <div class="tc-scan__main">
            <span class="tc-scan__num mono">{{ SCAN_TIMEOUT - scanSeconds }}</span>
            <div class="tc-scan__text">
              <strong>正在扫描局域网世界</strong>
              <span>请先在游戏里进入存档，按 Esc 点「对局域网开放」。</span>
            </div>
          </div>
          <WoodButton variant="ghost" size="sm" @click="cancelScan(false)">停止扫描</WoodButton>
        </div>

        <div v-else-if="tcStateName === 'host-starting'" class="tc-state">
          <span class="mp-pulse" />
          正在创建房间…
        </div>

        <div v-else-if="tcStateName === 'host-ok'" class="tc-room">
          <div class="tc-room__label mono">房间码</div>
          <button class="tc-code mono" title="点击复制邀请码" @click="copyAddress(tcRoom)">
            {{ tcRoom }}
          </button>
          <div v-if="tcProfiles.length" class="tc-members">
            <span v-for="profile in tcProfiles" :key="profile.machine_id" class="tc-member mono">
              {{ profile.kind === "HOST" ? "HOST" : "GUEST" }} · {{ profile.name }}
            </span>
          </div>
          <div class="tc-actions">
            <WoodButton variant="soft" size="sm" @click="copyAddress(tcRoom)">
              <AppIcon name="copy" :size="12" />
              复制邀请码
            </WoodButton>
            <WoodButton variant="ghost" size="sm" @click="leaveTc">关闭房间</WoodButton>
          </div>
        </div>

        <div
          v-else-if="tcStateName === 'guest-connecting' || tcStateName === 'guest-starting'"
          class="tc-state tc-state--col"
        >
          <div class="tc-state__line">
            <span class="mp-pulse" />
            {{ tcStateName === "guest-connecting" ? "正在连接房主…" : tcDifficulty }}
          </div>
          <span v-if="tcTough" class="tc-hint">
            网络较难直连，可在「设置 → 联机」里添加 EasyTier 公共节点后重新加入
          </span>
        </div>

        <div v-else-if="tcStateName === 'guest-ok'" class="tc-room">
          <div class="tc-room__label mono">直连地址</div>
          <button class="tc-code mono" title="点击复制直连地址" @click="copyAddress(tcUrl)">
            {{ tcUrl }}
          </button>
          <div v-if="tcProfiles.length" class="tc-members">
            <span v-for="profile in tcProfiles" :key="profile.machine_id" class="tc-member mono">
              {{ profile.kind === "HOST" ? "HOST" : "GUEST" }} · {{ profile.name }}
            </span>
          </div>
          <div class="tc-actions">
            <WoodButton variant="primary" size="sm" @click="join(tcUrl)">
              <AppIcon name="play" :size="12" />
              启动游戏加入
            </WoodButton>
            <WoodButton variant="ghost" size="sm" @click="leaveTc">离开房间</WoodButton>
          </div>
        </div>

        <div v-else-if="tcStateName === 'exception'" class="tc-alert">
          <span class="tc-alert__text">{{ tcExceptionText }}</span>
          <WoodButton
            variant="primary"
            size="sm"
            :disabled="tcBusy || !tcRetryRoom"
            @click="retryTc"
          >
            重新加入
          </WoodButton>
          <WoodButton variant="ghost" size="sm" @click="leaveTc">返回</WoodButton>
        </div>

        <div v-else class="tc-state">
          <span class="mp-pulse" />
          正在同步房间状态…
        </div>
      </template>

      <div v-else class="tc-idle">
        <WoodButton variant="primary" size="md" @click="prepareTc">启动陶瓦联机</WoodButton>
      </div>

      <div class="tc-credit">
        Terracotta / 陶瓦联机 by burningtnt · AGPL-3.0
        <button @click="openExternal('https://github.com/burningtnt/Terracotta')">项目主页</button>
      </div>

      <div v-if="recents.length" class="tc-recents">
        <div class="tc-recents__head">
          <span class="tc-recents__title">最近一起玩的人</span>
          <WoodButton variant="ghost" size="sm" @click="clearRecents">清空</WoodButton>
        </div>
        <div v-for="person in recents" :key="person.machine_id" class="tc-recent">
          <span class="tc-recent__name">{{ person.name }}</span>
          <span class="tc-recent__room mono">{{ person.room }}</span>
          <WoodButton variant="soft" size="sm" @click="copyAddress(person.room)">
            复制邀请码
          </WoodButton>
        </div>
        <p class="tc-hint">邀请码只在房主重新开房前有效；陶瓦没有好友接口，这里只记录本机见过的玩家。</p>
      </div>
    </section>

    <section v-else-if="tab === 'direct'" class="card card-pad mp-panel reveal">
      <div class="mp-panel__head">
        <div>
          <div class="section-title">直连服务器</div>
        </div>
      </div>

      <p class="muted small mp-hint">
        支持域名或 IP，端口默认 25565；1.20 及以上会使用快速加入参数，旧版本走 --server。
      </p>

      <div class="mp-direct">
        <WoodInput
          v-model="directAddress"
          mono
          placeholder="play.example.com:25565"
          @keyup.enter="joinDirect"
        />
        <WoodButton
          variant="primary"
          size="md"
          :disabled="!!joining"
          @click="joinDirect"
        >
          <AppIcon name="play" :size="14" />
          {{ joining ? "连接中…" : "加入" }}
        </WoodButton>
      </div>

      <p class="muted small mp-hint">当前版本：{{ currentVersionLabel }}</p>
    </section>

    <section v-else-if="tab === 'recent'" class="card card-pad mp-panel reveal">
      <div class="mp-panel__head">
        <div>
          <div class="section-title">最近服务器</div>
        </div>
        <IconButton
          icon="refresh"
          label="刷新服务器状态"
          :size="32"
          :disabled="pinging"
          @click="pingAll"
        />
      </div>

      <div class="mp-direct">
        <WoodInput
          v-model="favoriteAddress"
          mono
          placeholder="收藏服务器地址，例如 play.example.com"
          @keyup.enter="addFavorite"
        />
        <WoodButton variant="soft" size="md" @click="addFavorite">
          <AppIcon name="plus" :size="14" />
          收藏
        </WoodButton>
      </div>

      <div v-if="!recentServers.length" class="mp-empty mp-empty--card">
        <span class="mp-empty__icon">
          <AppIcon name="clock" :size="20" />
        </span>
        <div class="mp-empty__text">
          <strong>还没有服务器记录</strong>
          <span class="muted small">
            在上方收藏一个地址，或通过「直连地址」加入一次服务器后，这里会显示它的在线状态、人数和延迟。
          </span>
        </div>
        <WoodButton variant="soft" size="sm" @click="tab = 'direct'">去直连</WoodButton>
      </div>
      <div v-else class="mp-list">
        <div
          v-for="server in recentServers"
          :key="server.address"
          class="mp-room"
          :title="tipOf(server.address)"
        >
          <span class="mp-room__icon" :class="iconClass(server.address)">
            <img
              v-if="faviconOf(server.address)"
              :src="faviconOf(server.address)"
              class="mp-room__favicon"
              alt=""
            />
            <AppIcon v-else name="cube" :size="18" />
          </span>
          <span class="mp-room__text">
            <strong>{{ titleOf(server) }}</strong>
            <span class="mono">{{ stateLine(server) }}</span>
          </span>
          <span class="mp-room__time mono">{{ usedLabel(server.last_used) }}</span>
          <IconButton
            icon="trash"
            label="从最近服务器移除"
            variant="danger-quiet"
            :size="30"
            @click="removeRecent(server.address)"
          />
          <WoodButton
            variant="primary"
            size="sm"
            :disabled="!!joining"
            @click="join(server.address)"
          >
            <AppIcon name="play" :size="12" />
            加入
          </WoodButton>
        </div>
      </div>
    </section>

    <section v-else class="card card-pad mp-panel reveal">
      <template v-if="serverStatus">
        <div class="mp-panel__head">
          <div class="mp-server__state">
            <span class="mp-dot" :class="{ 'mp-dot--on': serverStatus.online }" />
            <div>
              <div class="section-title">
                {{
                  serverStatus.attached
                    ? "已接管上次运行的服务端"
                    : serverStatus.online
                      ? "服务端已就绪"
                      : "服务端启动中…"
                }}
              </div>
            </div>
            <span class="muted small">
              {{ serverVersionLabel(serverStatus.version_id) }} ·
              {{ loaderLabels[serverStatus.loader] ?? serverStatus.loader }} ·
              {{ serverUptime }}
            </span>
            <span v-if="serverStatus.attached" class="tag">启动器已接管</span>
          </div>
          <div class="mp-bar__right">
            <IconButton icon="folder" label="打开服务端目录" :size="32" @click="openServerDir" />
            <WoodButton variant="danger" size="sm" @click="stopServer">
              <AppIcon name="stop" :size="12" />
              停止
            </WoodButton>
          </div>
        </div>

        <div class="mp-address">
          <button class="mp-address__item" @click="copyAddress(serverStatus.lan_address)">
            <span class="muted small">局域网</span>
            <strong class="mono">{{ serverStatus.lan_address }}</strong>
          </button>
          <button class="mp-address__item" @click="copyAddress(serverStatus.local_address)">
            <span class="muted small">本机</span>
            <strong class="mono">{{ serverStatus.local_address }}</strong>
          </button>
          <span class="muted small">
            {{
              serverStatus.attached
                ? "启动器重启后接管，控制台输出不可用；仍可复制地址或停止"
                : "点地址复制；关闭启动器不会停止服务端"
            }}
          </span>
        </div>

        <div class="mp-ready">
          <span v-if="!serverStatus.online_mode" class="mp-badge mp-badge--warn">
            离线模式
          </span>
          <span v-else class="mp-badge">正版验证</span>
          <span v-if="serverStatus.white_list.length" class="mp-badge">
            白名单：{{ serverStatus.white_list.join("、") }}
          </span>
          <span v-else-if="serverStatus.attached" class="mp-badge mp-badge--warn">
            白名单为空
          </span>
        </div>

        <div v-if="serverStatus.attached" class="mp-attached">
          <AppIcon name="alert" :size="14" />
          这个服务端是上次退出启动器前启动的。控制台输出和命令输入无法恢复，但端口、地址和停止操作仍然有效。
        </div>

        <template v-else>
          <div class="mp-console__bar">
            <span class="muted small">控制台只接受服务端指令，Java 参数请到设置里填写</span>
            <WoodButton variant="ghost" size="sm" :disabled="reportBusy" @click="makeReport">
              {{ reportBusy ? "生成中…" : "生成诊断报告" }}
            </WoodButton>
          </div>
          <div ref="consoleEl" class="mp-console">
            <template v-for="(item, index) in consoleLines" :key="index">
              <div
                class="mp-console__line"
                :class="item.note ? `mp-console__line--${item.note.level}` : ''"
              >
                {{ item.line }}
              </div>
              <div
                v-if="item.note"
                class="mp-console__note"
                :class="`mp-console__note--${item.note.level}`"
              >
                <span>{{ item.note.text }}</span>
                <button
                  v-if="item.note.action"
                  class="mp-console__act"
                  @click="runNoteAction(item.note.action!)"
                >
                  {{ item.note.action.label }}
                </button>
              </div>
            </template>
          </div>

          <div class="mp-direct">
            <WoodInput
              v-model="consoleInput"
              mono
              placeholder="输入命令，例如 list / say hi / stop"
              @keyup.enter="sendConsole"
            />
            <WoodButton variant="soft" size="md" @click="sendConsole">发送</WoodButton>
          </div>
          <p v-if="consoleHint" class="mp-hint mp-hint--warn mp-console__hint">
            <template v-if="consoleHint === 'jvm'">
              「{{ consoleInput }}」是 Java 虚拟机参数，需要写到「设置 → 性能与参数 → Java 虚拟机参数」，重启服务端后生效。
            </template>
            <template v-else-if="consoleHint === 'cjk'">
              控制台只能输入英文服务端指令；要发中文聊天内容请用 say / tell。
            </template>
            <template v-else>
              spark 不是原版服务端指令；如果确实装了 spark 插件，可以继续发送。
            </template>
            <button
              v-if="consoleHint === 'jvm'"
              class="mp-console__act"
              @click="openPerformanceSettings"
            >
              去设置
            </button>
          </p>
        </template>
      </template>

      <template v-else>
        <div class="mp-panel__head">
          <div>
            <div class="section-title">本地开服</div>
          </div>
        </div>

        <p class="muted small mp-hint">
          支持原版与 Fabric / Forge / NeoForge；首次开服会下载服务端或运行加载器安装器，端口被占用会自动向后顺延。
        </p>

        <div v-if="!serverVersions.length" class="mp-empty">
          还没有可开服的版本，先去安装一个原版或带加载器的版本。
        </div>

        <template v-else>
          <div class="mp-form">
            <div class="mp-field">
              <span class="mp-field__label">使用版本</span>
              <div class="mp-field__row">
                <WoodSelect v-model="serverVersionId" :options="serverVersionOptions" />
                <IconButton
                  v-if="selectedServerIsPack"
                  icon="trash"
                  label="删除服务端包"
                  variant="danger-quiet"
                  :size="32"
                  @click="removeServerPack"
                />
              </div>
            </div>
            <div class="mp-field">
              <span class="mp-field__label">端口</span>
              <WoodInput
                :model-value="String(serverForm.port)"
                @update:model-value="serverForm.port = Number($event) || 25565"
              />
            </div>
            <div class="mp-field mp-field--wide">
              <span class="mp-field__label">MOTD</span>
              <WoodInput v-model="serverForm.motd" placeholder="显示在多人列表里的服务器名" />
            </div>
            <div class="mp-field">
              <span class="mp-field__label">最大玩家</span>
              <WoodInput
                :model-value="String(serverForm.maxPlayers)"
                @update:model-value="serverForm.maxPlayers = Number($event) || 10"
              />
            </div>
            <div class="mp-field">
              <span class="mp-field__label">视距</span>
              <WoodInput
                :model-value="String(serverForm.viewDistance)"
                @update:model-value="serverForm.viewDistance = Number($event) || 8"
              />
            </div>
            <div class="mp-field">
              <span class="mp-field__label">你的游戏 ID</span>
              <WoodInput v-model="serverForm.playerName" placeholder="开启白名单时自动加入" />
            </div>
          </div>

          <div class="mp-toggles">
            <div class="mp-toggle">
              <span>正版验证</span>
              <WoodToggle v-model="serverForm.onlineMode" />
            </div>
            <div class="mp-toggle">
              <span>白名单</span>
              <WoodToggle v-model="serverForm.whiteList" />
            </div>
            <div class="mp-toggle mp-toggle--eula">
              <span>我已阅读并同意 Minecraft EULA</span>
              <WoodToggle v-model="serverForm.eula" />
            </div>
          </div>

          <p v-if="serverForm.onlineMode && accountIsOffline" class="muted small mp-hint mp-hint--warn">
            当前登录的是离线账号，开启正版验证后会被服务端拒绝。自己或朋友都离线玩时，请关闭「正版验证」。
          </p>
          <p v-if="!serverForm.onlineMode" class="muted small mp-hint mp-hint--warn">
            已关闭正版验证：任何知道地址的人都能用任意 ID 进入。只建议自己或同一局域网内使用，开公网请改回正版验证。
          </p>
          <p v-if="serverForm.whiteList && !serverForm.playerName.trim()" class="muted small mp-hint mp-hint--warn">
            已开启白名单但没填「你的游戏 ID」，空白的白名单会让所有人都进不来。填上后开服会自动加入，注意大小写要和游戏内登录名完全一致。
          </p>

          <p v-if="selectedServerLoader === 'quilt'" class="muted small mp-hint">
            Quilt 服务端暂未支持，请改用 Fabric / Forge / NeoForge。
          </p>
          <p v-else-if="selectedServerLoader !== 'vanilla'" class="muted small mp-hint">
            当前选择的是 {{ loaderLabels[selectedServerLoader] ?? selectedServerLoader }} 版本，首次开服会运行加载器安装器，耗时取决于网络。
          </p>

          <div class="mp-bar__right">
            <WoodButton
              variant="primary"
              size="md"
              :disabled="serverBusy"
              @click="startServer"
            >
              <AppIcon name="play" :size="14" />
              {{ serverBusy ? "准备中…（首次下载 / 安装加载器）" : "启动服务端" }}
            </WoodButton>
          </div>
        </template>
      </template>
    </section>
      </div>
    </div>
  </div>

  <WoodModal
    v-if="reportOpen"
    title="服务端诊断报告"
    :width="640"
    @close="reportOpen = false"
  >
    <p class="muted small mp-report__hint">
      已包含版本、内存、关键配置、白名单和最近 40 行日志。复制后可直接发给客服或其它 AI，省去反复追问。
    </p>
    <textarea class="mp-report" readonly :value="reportText" />
    <template #footer>
      <WoodButton variant="ghost" size="sm" @click="reportOpen = false">关闭</WoodButton>
      <WoodButton variant="primary" size="sm" @click="copyReport">复制全部</WoodButton>
    </template>
  </WoodModal>
</template>

<style scoped>
.multi {
  display: flex;
  flex-direction: column;
  gap: var(--space-6);
}

.mp-layout {
  display: grid;
  grid-template-columns: 208px minmax(0, 1fr);
  gap: var(--space-6);
  align-items: start;
}

.mp-side {
  display: flex;
  flex-direction: column;
  gap: var(--space-6);
  padding: var(--space-4);
  /* 和实例页一致：整列当一只托盘，外框 2px，里面衬一圈受光细边 */
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--panel) 46%, #fff);
}

.mp-side__group {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.mp-side__group + .mp-side__group {
  padding-top: 12px;
  border-top: 1px dashed var(--line);
}

.mp-side .side-label {
  padding: 0 10px 6px;
}

.mp-nav {
  display: flex;
  align-items: center;
  gap: 9px;
  width: 100%;
  height: 34px;
  padding: 0 10px;
  border-radius: 0;
  color: var(--ink-soft);
  font-size: var(--text-base);
  font-weight: 600;
  text-align: left;
  transition: background var(--dur-1) ease, color var(--dur-1) ease,
    box-shadow var(--dur-1) ease;
}

.mp-nav > svg {
  color: var(--muted);
  flex-shrink: 0;
  transition: color var(--dur-1) ease;
}

.mp-nav > span:not(.mp-nav__count) {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.mp-nav:hover {
  background: var(--hover);
  color: var(--walnut-deep);
}

.mp-nav:hover > svg {
  color: var(--walnut);
}

.mp-nav--active,
.mp-nav--active:hover {
  background: var(--moss-soft);
  color: var(--moss);
  /* 选中项左侧压一条 3px 实心竖条，和实例页一致 */
  box-shadow: inset 3px 0 0 var(--moss);
}

.mp-nav--active > svg,
.mp-nav--active:hover > svg {
  color: var(--moss);
}

.mp-nav__count {
  flex: none;
  min-width: 18px;
  padding: 1px 5px;
  border-radius: 0;
  background: var(--panel);
  border: 1px solid var(--line);
  color: var(--muted);
  font-size: 9.5px;
  text-align: center;
}

.mp-nav--active .mp-nav__count {
  background: color-mix(in srgb, var(--moss) 18%, var(--panel));
  border-color: color-mix(in srgb, var(--moss) 40%, transparent);
  color: var(--moss);
}

.mp-content {
  display: flex;
  flex-direction: column;
  gap: 14px;
  min-width: 0;
}

.mp-context {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-6);
  padding: 10px 16px;
}

.mp-context__text {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
}

.mp-context__text strong {
  font-size: var(--text-lg);
  font-weight: 700;
  color: var(--walnut-deep);
}

.mp-bar__right {
  display: flex;
  align-items: center;
  gap: var(--space-5);
  flex-wrap: wrap;
}

.mp-panel {
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
}

.mp-panel__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-5);
}

.mp-hint {
  margin: 0;
}

.mp-alert {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: 9px 12px;
  border-radius: 0;
  border: 1px solid color-mix(in srgb, var(--danger) 34%, var(--line-strong));
  background: var(--danger-soft);
  /* 告警条做成浅凹槽：内容嵌在面板里，而不是浮在上面 */
  box-shadow: inset 0 1px 2px rgba(90, 40, 30, 0.12);
  color: var(--danger);
  font-size: var(--text-base);
}

.mp-empty {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 22px 4px;
  color: var(--muted);
  font-size: var(--text-base);
}

.mp-empty--card {
  padding: 16px 16px;
  border-radius: 2px;
  border: 1px dashed var(--line-strong);
  background: var(--panel-2);
}

.mp-empty__icon {
  display: grid;
  place-items: center;
  width: 38px;
  height: 38px;
  border-radius: 0;
  background: var(--moss-soft);
  color: var(--moss);
  flex-shrink: 0;
}

.mp-empty__text {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.mp-empty__text strong {
  font-size: var(--text-base);
  font-weight: 700;
  color: var(--walnut-deep);
}

/* ---------- 陶瓦房间 ---------- */
.tc-panel {
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
  padding: 0 16px 16px;
  border-radius: 0;
  overflow: hidden;
}

.tc-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-5);
  flex-shrink: 0;
  height: 44px;
  margin: 0 -16px;
  padding: 0 16px;
  border-bottom: 1px solid var(--line);
}

.tc-head__title {
  font-size: var(--text-lg);
  font-weight: 700;
  color: var(--walnut-deep);
}

.tc-head__state {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  font-size: var(--text-xs);
  letter-spacing: 0.08em;
  color: var(--muted);
}

.tc-dot {
  width: 6px;
  height: 6px;
  background: var(--line-strong);
}

.tc-dot--on {
  background: var(--moss);
  box-shadow: 0 0 6px rgba(var(--accent-rgb), 0.5);
}

.tc-version {
  color: var(--muted);
}

.tc-desc {
  padding-top: 12px;
  max-width: 72ch;
  color: var(--muted);
  font-size: var(--text-sm);
  line-height: 1.6;
}

.tc-state {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  padding: 14px;
  border: 1px solid var(--line-strong);
  background: rgba(var(--glass-rgb), 0.28);
  box-shadow: inset 0 1px 2px rgba(60, 40, 20, 0.08);
  color: var(--ink-soft);
  font-size: var(--text-base);
}

.tc-alert {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  padding: 10px 12px;
  border: 1px solid color-mix(in srgb, var(--danger) 34%, var(--line-strong));
  background: var(--danger-soft);
  box-shadow: inset 0 1px 2px rgba(90, 40, 30, 0.12);
  color: var(--danger);
  font-size: var(--text-sm);
}

.tc-alert__text {
  flex: 1;
  min-width: 0;
  word-break: break-word;
}

.tc-form {
  display: grid;
  grid-template-columns: 220px minmax(0, 1fr);
  gap: var(--space-5);
}

.tc-field {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  min-width: 0;
}

.tc-field__label {
  color: var(--ink-soft);
  font-size: var(--text-sm);
  font-weight: 600;
}

.tc-actions {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-3);
}

.tc-hint {
  color: var(--muted);
  font-size: var(--text-sm);
  line-height: 1.6;
}

.tc-state--col {
  flex-direction: column;
  align-items: flex-start;
  gap: var(--space-3);
}

.tc-state__line {
  display: flex;
  align-items: center;
  gap: var(--space-4);
}

.tc-recents {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  margin-top: 14px;
  padding-top: 12px;
  border-top: 1px solid var(--line);
}

.tc-recents__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
}

.tc-recents__title {
  color: var(--ink-soft);
  font-size: var(--text-sm);
  font-weight: 600;
}

.tc-recent {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  padding: 6px 8px;
  border: 1px solid var(--line);
  background: rgba(var(--glass-rgb), 0.22);
  font-size: var(--text-sm);
}

.tc-recent__name {
  flex-shrink: 0;
  color: var(--ink);
}

.tc-recent__room {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  color: var(--muted);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.tc-scan {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-6);
  padding: 14px;
  border: 1px solid var(--line);
  background: rgba(var(--glass-rgb), 0.28);
}

.tc-scan__main {
  display: flex;
  align-items: center;
  gap: 14px;
  min-width: 0;
}

.tc-scan__num {
  min-width: 42px;
  font-size: 28px;
  font-weight: 700;
  line-height: 1;
  color: var(--walnut-deep);
}

.tc-scan__text {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.tc-scan__text strong {
  font-size: var(--text-base);
  color: var(--walnut-deep);
}

.tc-scan__text span {
  font-size: var(--text-sm);
  color: var(--muted);
}

.tc-room {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
  padding: 14px;
  border: 1px solid var(--line-strong);
  background: rgba(var(--glass-rgb), 0.28);
  box-shadow: inset 0 1px 2px rgba(60, 40, 20, 0.07);
}

.tc-room__label {
  font-size: 9.5px;
  letter-spacing: 0.14em;
  color: var(--muted);
}

.tc-code {
  align-self: flex-start;
  padding: 4px 0;
  border-bottom: 1px solid var(--line-strong);
  color: var(--walnut-deep);
  font-size: var(--text-2xl);
  font-weight: 700;
  letter-spacing: 0.08em;
  user-select: text;
  transition: color var(--dur-1) ease, border-color var(--dur-1) ease;
}

.tc-code:hover {
  color: var(--moss);
  border-color: var(--moss);
}

.tc-members {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
}

.tc-member {
  padding: 2px 7px;
  border: 1px solid var(--line);
  background: transparent;
  color: var(--ink-soft);
  font-size: var(--text-xs);
}

.tc-idle {
  display: flex;
  justify-content: flex-start;
  padding: 4px 0;
}

.tc-credit {
  margin-top: auto;
  padding-top: 10px;
  border-top: 1px solid var(--line);
  color: var(--muted);
  font-size: var(--text-xs);
}

.tc-credit button {
  margin-left: 6px;
  color: var(--moss);
  font-size: var(--text-xs);
}

.tc-credit button:hover {
  text-decoration: underline;
}

.mp-pulse {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--moss-bright);
  box-shadow: 0 0 0 3px rgba(136, 168, 103, 0.16);
  animation: mp-pulse 1.8s var(--ease-in-out) infinite;
}

@keyframes mp-pulse {
  0%,
  100% {
    opacity: 1;
    transform: scale(1);
  }
  50% {
    opacity: 0.45;
    transform: scale(0.8);
  }
}

.mp-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.mp-room {
  display: flex;
  align-items: center;
  gap: var(--space-5);
  padding: 9px 12px;
  border-radius: 0;
  border-style: solid;
  border-width: 2px;
  border-top-color: color-mix(in srgb, var(--frame) 66%, #fff);
  border-left-color: color-mix(in srgb, var(--frame) 66%, #fff);
  border-right-color: color-mix(in srgb, var(--frame) 74%, #000);
  border-bottom-color: color-mix(in srgb, var(--frame) 74%, #000);
  background: var(--panel);
  transition: transform var(--dur-2) var(--ease-out-expo), box-shadow var(--dur-2) ease;
}

.mp-room:hover {
  transform: translateY(-1px);
  box-shadow: var(--shadow-1);
}

.mp-room__icon {
  display: grid;
  place-items: center;
  width: 34px;
  height: 34px;
  border-radius: 0;
  background: var(--moss-soft);
  color: var(--moss);
  flex-shrink: 0;
  overflow: hidden;
  transition: background var(--dur-1) ease, color var(--dur-1) ease;
}

.mp-room__icon--on {
  background: var(--moss-soft);
  color: var(--moss);
  box-shadow: inset 0 0 0 1px rgba(136, 168, 103, 0.35);
}

.mp-room__icon--off {
  background: var(--danger-soft);
  color: var(--danger);
  box-shadow: inset 0 0 0 1px rgba(176, 96, 82, 0.28);
}

.mp-room__favicon {
  width: 100%;
  height: 100%;
  object-fit: contain;
  image-rendering: pixelated;
}

.mp-room__text {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.mp-room__text strong {
  font-size: var(--text-lg);
  color: var(--walnut-deep);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.mp-room__text span {
  font-size: var(--text-sm);
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.mp-room__time {
  font-size: var(--text-xs);
  color: var(--muted);
  flex-shrink: 0;
}

.mp-direct {
  display: flex;
  align-items: center;
  gap: var(--space-4);
}

.mp-direct .input {
  flex: 1;
  min-width: 0;
}

.mp-server__state {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  min-width: 0;
}

.mp-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--amber);
  box-shadow: 0 0 0 3px rgba(217, 160, 91, 0.18);
  animation: mp-pulse 1.8s var(--ease-in-out) infinite;
  flex-shrink: 0;
}

.mp-dot--on {
  background: var(--moss-bright);
  box-shadow: 0 0 0 3px rgba(136, 168, 103, 0.18);
  animation: none;
}

.mp-address {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--space-3);
  padding: 8px 10px;
  border-radius: 0;
  border: 1px dashed var(--line-strong);
  background: var(--panel-2);
}

.mp-address__item {
  display: flex;
  flex-direction: column;
  gap: 1px;
  padding: 4px 8px;
  border-radius: 0;
  text-align: left;
  transition: background var(--dur-1) ease;
}

.mp-address__item:hover {
  background: rgba(var(--accent-rgb), 0.07);
}

.mp-address__item strong {
  font-size: var(--text-base);
  color: var(--walnut-deep);
}

.mp-address > .muted {
  margin-left: auto;
}

.mp-attached {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: 10px 12px;
  border-radius: 0;
  border: 1px dashed var(--line-strong);
  background: var(--panel-2);
  color: var(--ink-soft);
  font-size: var(--text-base);
  line-height: 1.55;
}

.mp-console {
  height: 220px;
  overflow-y: auto;
  border-radius: 0;
  border-style: solid;
  border-width: 1px;
  border-top-color: color-mix(in srgb, var(--line-strong) 64%, #000);
  border-left-color: color-mix(in srgb, var(--line-strong) 64%, #000);
  border-right-color: color-mix(in srgb, var(--line-strong) 38%, #fff);
  border-bottom-color: color-mix(in srgb, var(--line-strong) 38%, #fff);
  background: var(--paper-2);
  box-shadow: inset 0 1px 2px rgba(60, 40, 20, 0.1);
  padding: 9px 12px;
  font-family: var(--font-mono);
  font-size: var(--text-sm);
  line-height: 1.55;
  color: var(--ink-soft);
  user-select: text;
}

.mp-console__line {
  white-space: pre-wrap;
  word-break: break-word;
}

.mp-console__line--info {
  color: var(--muted);
}

.mp-console__line--warn {
  color: color-mix(in srgb, var(--amber) 55%, var(--walnut-deep));
}

.mp-console__line--error {
  color: var(--danger);
}

/* 已知日志的中文注解：紧贴在原行下面，像给日志加了一条脚注 */
.mp-console__note {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-3);
  margin: 2px 0 7px;
  padding: 5px 9px;
  border-left: 2px solid var(--line-strong);
  background: color-mix(in srgb, var(--panel-2) 72%, transparent);
  color: var(--ink-soft);
  font-family: var(--font-ui);
  font-size: var(--text-sm);
  line-height: 1.5;
}

.mp-console__note--warn {
  border-left-color: var(--amber);
  background: color-mix(in srgb, var(--amber) 14%, var(--panel-2));
}

.mp-console__note--error {
  border-left-color: var(--danger);
  background: var(--danger-soft);
  color: color-mix(in srgb, var(--danger) 82%, var(--ink));
}

.mp-console__act {
  padding: 1px 7px;
  border: 1px dashed currentColor;
  border-radius: 0;
  color: var(--moss);
  font-size: var(--text-sm);
  font-weight: 600;
  transition: background var(--dur-1) ease;
}

.mp-console__act:hover {
  background: var(--moss-soft);
}

.mp-console__bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-5);
  margin: 12px 0 6px;
}

.mp-console__hint {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-3);
}

/* 开服准备状态：离线模式 / 白名单是否已经把人装进去 */
.mp-ready {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
}

.mp-badge {
  display: inline-flex;
  align-items: center;
  height: 22px;
  padding: 0 9px;
  border-radius: 0;
  border: 1px solid color-mix(in srgb, var(--moss) 32%, var(--line-strong));
  background: var(--moss-soft);
  color: var(--moss);
  font-size: var(--text-sm);
  font-weight: 600;
}

.mp-badge--warn {
  border-color: color-mix(in srgb, var(--amber) 46%, var(--line-strong));
  background: color-mix(in srgb, var(--amber) 18%, var(--panel-2));
  color: color-mix(in srgb, var(--amber) 48%, var(--walnut-deep));
}

.mp-hint--warn {
  padding: 7px 10px;
  border-left: 2px solid var(--amber);
  background: color-mix(in srgb, var(--amber) 12%, var(--panel-2));
  color: color-mix(in srgb, var(--amber) 48%, var(--walnut-deep));
  line-height: 1.55;
}

.mp-report__hint {
  margin: 0 0 10px;
}

.mp-report {
  width: 100%;
  height: 46vh;
  min-height: 260px;
  resize: vertical;
  padding: 10px 12px;
  border-radius: 0;
  border: 1px solid var(--line-strong);
  background: var(--paper-2);
  color: var(--ink-soft);
  font-family: var(--font-mono);
  font-size: var(--text-sm);
  line-height: 1.55;
  user-select: text;
}

.mp-form {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 10px 12px;
}

.mp-field {
  display: flex;
  flex-direction: column;
  gap: 5px;
  min-width: 0;
}

.mp-field--wide {
  grid-column: span 2;
}

.mp-field__label {
  font-size: var(--text-sm);
  color: var(--muted);
}

.mp-field__row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  min-width: 0;
}

.mp-field__row > :first-child {
  flex: 1;
  min-width: 0;
}

.mp-field__row > :last-child {
  flex-shrink: 0;
}

.mp-toggles {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-6);
}

.mp-tc-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-4);
}

.mp-tc-room {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: var(--space-3);
}

.mp-room-code {
  font-size: var(--text-2xl);
  letter-spacing: 0.08em;
  color: var(--walnut-deep);
  background: var(--panel-2);
  border: 1px dashed var(--line-strong);
  border-radius: 0;
  padding: 7px 14px;
  transition: border-color var(--dur-1) ease, background var(--dur-1) ease;
}

.mp-room-code:hover {
  border-color: var(--moss);
  background: var(--moss-soft);
}

.mp-members {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
}

.mp-member {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  height: 25px;
  padding: 0 10px;
  border-radius: 0;
  background: var(--moss-soft);
  color: var(--moss);
  font-size: var(--text-sm);
  font-weight: 600;
}

.mp-credit {
  padding-top: 10px;
  border-top: 1px dashed var(--line);
  font-size: var(--text-xs);
  color: var(--muted);
}

.mp-credit button {
  color: var(--moss);
  font-size: inherit;
  text-decoration: underline;
  padding: 0;
}

.mp-toggle {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  font-size: var(--text-base);
  color: var(--ink-soft);
}

.mp-toggle--eula {
  padding-left: 16px;
  border-left: 1px dashed var(--line-strong);
}

@media (max-width: 980px) {
  .mp-layout {
    grid-template-columns: 1fr;
  }

  .mp-side {
    flex-direction: row;
    flex-wrap: wrap;
    gap: 8px 16px;
    padding: 10px 12px;
  }

  .mp-side__group {
    flex-direction: row;
    align-items: center;
    flex-wrap: wrap;
    gap: 2px;
  }

  .mp-side__group + .mp-side__group {
    padding-top: 0;
    border-top: none;
  }

  .mp-side__group .side-label {
    padding: 0 4px 0 0;
  }

  .mp-nav {
    width: auto;
    padding: 0 10px;
  }
}

@media (max-width: 900px) {
  .mp-form {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .mp-field--wide {
    grid-column: span 2;
  }

  .mp-toggle--eula {
    width: 100%;
    padding-left: 0;
    border-left: none;
  }
}
</style>

