<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  api,
  openExternal,
  type AccountView,
  type CapeView,
  type SkinLibraryItem,
} from "../api";
import AppIcon from "../components/AppIcon.vue";
import IconButton from "../components/IconButton.vue";
import LittleSkinPanel from "../components/LittleSkinPanel.vue";
import SkinModel from "../components/SkinModel.vue";
import WoodButton from "../components/WoodButton.vue";
import WoodInput from "../components/WoodInput.vue";
import WoodModal from "../components/WoodModal.vue";
import WoodSelect from "../components/WoodSelect.vue";
import WoodToggle from "../components/WoodToggle.vue";
import { skinPortrait } from "../skinPreview";
import { loadSelectedSkin, notify, refreshAccounts, store } from "../store";

const items = ref<SkinLibraryItem[]>([]);
const loading = ref(true);
const error = ref("");

const importOpen = ref(false);
const importName = ref("");
const importModel = ref("classic");
const importPath = ref("");
const importing = ref(false);

const harvestOpen = ref(false);
const harvestName = ref("");
const harvestAccount = ref("");
const harvesting = ref(false);

const playerOpen = ref(false);
const playerName = ref("");
const playerFetching = ref(false);

const applyTarget = ref<SkinLibraryItem | null>(null);
const applyAccount = ref("");
const applyUpload = ref(false);
const applying = ref(false);

const renameTarget = ref<SkinLibraryItem | null>(null);
const renameName = ref("");
const renaming = ref(false);

const deleteTarget = ref<SkinLibraryItem | null>(null);
const deleting = ref(false);

const capeAccount = ref("");
const capes = ref<CapeView[]>([]);
const capesLoading = ref(false);
const capesError = ref("");
const capeBusy = ref("");

/// 右栏页签：本地皮肤库 / LittleSkin 衣柜 / 披风
const tab = ref<"local" | "littleskin" | "capes">("local");
const keyword = ref("");
/// 左栏预览：静止或行走
const walking = ref(true);
const previewId = ref("");
/// 卡片缩略图（本地拼好的 16×32 人形立绘）
const portraits = ref<Record<string, string>>({});
let portraitToken = 0;

const filteredItems = computed(() => {
  const key = keyword.value.trim().toLowerCase();
  if (!key) return items.value;
  return items.value.filter((item) => item.name.toLowerCase().includes(key));
});
const previewItem = computed(
  () => items.value.find((item) => item.id === previewId.value) ?? null
);
const previewData = computed(
  () => previewItem.value?.data_url ?? store.accountSkin?.data_url ?? ""
);
const previewModel = computed(
  () => previewItem.value?.model ?? store.accountSkin?.model ?? "classic"
);
const previewSeed = computed(() => previewItem.value?.id ?? "account-skin");
const previewName = computed(() => previewItem.value?.name ?? "当前皮肤");
const previewSource = computed(() =>
  previewItem.value ? sourceLabel(previewItem.value.source) : store.accountSkin ? "账号" : ""
);

const accountOptions = computed(() =>
  store.accounts.map((account) => ({ value: account.id, label: account.name }))
);
/// 披风：微软账号可切换，第三方账号只读展示皮肤站下发的披风
const capeAccountOptions = computed(() =>
  store.accounts
    .filter((account) => account.kind === "microsoft" || account.kind === "authlib")
    .map((account) => ({ value: account.id, label: account.name }))
);
const capeAccountView = computed<AccountView | null>(
  () => store.accounts.find((account) => account.id === capeAccount.value) ?? null
);
const applyAccountView = computed<AccountView | null>(
  () => store.accounts.find((account) => account.id === applyAccount.value) ?? null
);

function sourceLabel(source: string) {
  if (source === "account") return "来自账号";
  if (source === "player") return "玩家抓取";
  if (source === "default") return "内置";
  return "导入";
}

async function loadLibrary() {
  loading.value = true;
  error.value = "";
  try {
    items.value = await api.skinLibraryList();
    syncPreviewSelection();
    void buildPortraits();
  } catch (err) {
    error.value = String(err);
  } finally {
    loading.value = false;
  }
}

/// 默认预览当前正在使用的皮肤；否则预览第一张
function syncPreviewSelection() {
  if (previewId.value && items.value.some((item) => item.id === previewId.value)) return;
  const current = items.value.find(isCurrent);
  previewId.value = current?.id ?? items.value[0]?.id ?? "";
}

function isCurrent(item: SkinLibraryItem) {
  return !!store.accountSkin?.data_url && store.accountSkin.data_url === item.data_url;
}

function selectItem(item: SkinLibraryItem) {
  previewId.value = item.id;
}

/// 用卡片渲染缩略图：一台画布拼几十张也很快，不用为每张卡开 WebGL
async function buildPortraits() {
  const token = ++portraitToken;
  const list = items.value;
  const entries = await Promise.all(
    list.map(async (item) => {
      try {
        return [item.id, await skinPortrait(item.data_url, item.model)] as const;
      } catch {
        return [item.id, ""] as const;
      }
    })
  );
  if (token !== portraitToken) return;
  portraits.value = Object.fromEntries(entries);
}

async function pickImportFile() {
  const picked = await openDialog({
    multiple: false,
    filters: [{ name: "Minecraft 皮肤", extensions: ["png"] }],
  });
  if (typeof picked === "string") {
    importPath.value = picked;
    if (!importName.value.trim()) {
      const file = picked.split(/[\\/]/).pop() ?? "";
      importName.value = file.replace(/\.png$/i, "");
    }
  }
}

async function confirmImport() {
  if (importing.value) return;
  if (!importPath.value) {
    notify("请先选择皮肤文件", "err");
    return;
  }
  importing.value = true;
  try {
    await api.skinLibraryImport(
      importName.value.trim() || "未命名皮肤",
      importModel.value,
      importPath.value
    );
    importOpen.value = false;
    importName.value = "";
    importPath.value = "";
    importModel.value = "classic";
    await loadLibrary();
    notify("皮肤已入库");
  } catch (err) {
    notify(`入库失败：${String(err)}`, "err");
  } finally {
    importing.value = false;
  }
}

function openHarvest() {
  if (!store.accounts.length) {
    notify("还没有账号，请先到账号页添加", "err");
    return;
  }
  harvestAccount.value = store.config?.selected_account ?? store.accounts[0].id;
  const account = store.accounts.find((a) => a.id === harvestAccount.value);
  harvestName.value = account ? `${account.name} 的皮肤` : "";
  harvestOpen.value = true;
}

async function confirmPlayer() {
  const name = playerName.value.trim();
  if (!name || playerFetching.value) return;
  playerFetching.value = true;
  try {
    const item = await api.skinLibraryFromPlayer(name);
    playerOpen.value = false;
    playerName.value = "";
    await loadLibrary();
    notify(`已抓取 ${item.name} 的皮肤`);
  } catch (err) {
    notify(`抓取失败：${String(err)}`, "err");
  } finally {
    playerFetching.value = false;
  }
}

async function confirmHarvest() {
  if (harvesting.value) return;
  harvesting.value = true;
  try {
    await api.skinLibraryFromAccount(
      harvestName.value.trim() || "未命名皮肤",
      harvestAccount.value
    );
    harvestOpen.value = false;
    await loadLibrary();
    notify("已收录到皮肤库");
  } catch (err) {
    notify(`收录失败：${String(err)}`, "err");
  } finally {
    harvesting.value = false;
  }
}

function openApply(item: SkinLibraryItem) {
  if (!store.accounts.length) {
    notify("还没有账号，请先到账号页添加", "err");
    return;
  }
  applyTarget.value = item;
  applyAccount.value = store.config?.selected_account ?? store.accounts[0].id;
  applyUpload.value = false;
}

async function confirmApply() {
  const target = applyTarget.value;
  if (!target || applying.value) return;
  applying.value = true;
  try {
    await api.skinLibraryApply(target.id, applyAccount.value, applyUpload.value);
    await refreshAccounts();
    await loadSelectedSkin();
    applyTarget.value = null;
    notify(
      applyUpload.value
        ? `已应用「${target.name}」并同步上传 Mojang`
        : `已应用「${target.name}」`
    );
  } catch (err) {
    notify(`应用失败：${String(err)}`, "err");
  } finally {
    applying.value = false;
  }
}

function openRename(item: SkinLibraryItem) {
  renameTarget.value = item;
  renameName.value = item.name;
}

async function confirmRename() {
  const target = renameTarget.value;
  if (!target || renaming.value) return;
  renaming.value = true;
  try {
    await api.skinLibraryRename(target.id, renameName.value.trim());
    renameTarget.value = null;
    await loadLibrary();
  } catch (err) {
    notify(`重命名失败：${String(err)}`, "err");
  } finally {
    renaming.value = false;
  }
}

async function confirmDelete() {
  const target = deleteTarget.value;
  if (!target || deleting.value) return;
  deleting.value = true;
  try {
    await api.skinLibraryRemove(target.id);
    deleteTarget.value = null;
    await loadLibrary();
    notify(`已删除「${target.name}」`);
  } catch (err) {
    notify(`删除失败：${String(err)}`, "err");
  } finally {
    deleting.value = false;
  }
}

async function loadCapes() {
  if (!capeAccount.value) {
    capes.value = [];
    return;
  }
  capesLoading.value = true;
  capesError.value = "";
  try {
    capes.value = await api.accountCapes(capeAccount.value);
  } catch (err) {
    capes.value = [];
    capesError.value = String(err);
  } finally {
    capesLoading.value = false;
  }
}

async function toggleCape(cape: CapeView) {
  if (capeBusy.value) return;
  capeBusy.value = cape.id;
  try {
    await api.capeSetActive(capeAccount.value, cape.active ? null : cape.id);
    await loadCapes();
    notify(cape.active ? "已卸下披风" : `已启用披风「${cape.alias}」`);
  } catch (err) {
    notify(`披风操作失败：${String(err)}`, "err");
  } finally {
    capeBusy.value = "";
  }
}

watch(capeAccount, () => {
  void loadCapes();
});

/// 第三方账号的披风只能在皮肤站切换，这里按 API 根推出站点首页
function openSkinSite() {
  const trimmed = (capeAccountView.value?.authlib_url ?? "").trim().replace(/\/+$/, "");
  if (!trimmed) return;
  const site = trimmed.endsWith("/api/yggdrasil")
    ? trimmed.slice(0, -"/api/yggdrasil".length)
    : trimmed;
  openExternal(site);
}

/// 打开指定账号所属皮肤站的衣柜页（上传皮肤只能在那里做）
function openAccountSite(accountId: string) {
  const account = store.accounts.find((item) => item.id === accountId);
  const trimmed = (account?.authlib_url ?? "").trim().replace(/\/+$/, "");
  if (!trimmed) {
    notify("这个账号没有记录皮肤站地址", "err");
    return;
  }
  const site = trimmed.endsWith("/api/yggdrasil")
    ? trimmed.slice(0, -"/api/yggdrasil".length)
    : trimmed;
  openExternal(`${site}/user/closet`);
}

onMounted(async () => {
  await loadLibrary();
  if (capeAccountOptions.value.length) {
    const selected = store.config?.selected_account;
    capeAccount.value = capeAccountOptions.value.some((o) => o.value === selected)
      ? (selected as string)
      : capeAccountOptions.value[0].value;
  }
});
</script>

<template>
  <div class="page skins">
    <div class="skin-stage">
      <aside class="preview card">
        <div class="preview__viewport">
          <SkinModel
            :skin="previewData"
            :seed="previewSeed"
            :model="previewModel"
            :walking="walking"
            :height="430"
          />
        </div>
        <div class="preview__seg">
          <button :class="{ 'preview__seg-btn--active': !walking }" @click="walking = false">
            静止
          </button>
          <button :class="{ 'preview__seg-btn--active': walking }" @click="walking = true">
            行走
          </button>
        </div>
        <p class="preview__tip">拖动旋转</p>
        <div class="preview__meta">
          <strong>{{ previewName }}</strong>
          <span class="muted small">
            {{ previewModel === "slim" ? "纤细" : "经典" }}{{
              previewSource ? ` · ${previewSource}` : ""
            }}
          </span>
        </div>
        <WoodButton
          variant="primary"
          size="sm"
          :disabled="!previewItem"
          @click="previewItem && openApply(previewItem)"
        >
          使用这张皮肤
        </WoodButton>
      </aside>

      <section class="stage-main">
        <header class="stage-head">
          <div class="tabs">
            <button
              class="tabs__btn"
              :class="{ 'tabs__btn--active': tab === 'local' }"
              @click="tab = 'local'"
            >
              本地皮肤库
              <span class="tabs__count mono">{{ items.length }}</span>
            </button>
            <button
              class="tabs__btn"
              :class="{ 'tabs__btn--active': tab === 'littleskin' }"
              @click="tab = 'littleskin'"
            >
              LittleSkin 衣柜
            </button>
            <button
              class="tabs__btn"
              :class="{ 'tabs__btn--active': tab === 'capes' }"
              @click="tab = 'capes'"
            >
              披风
            </button>
          </div>
          <div class="stage-tools">
            <label v-if="tab !== 'capes'" class="search">
              <AppIcon name="search" :size="14" />
              <input v-model="keyword" :placeholder="tab === 'local' ? '搜索皮肤' : '搜索衣柜'" />
            </label>
            <template v-if="tab === 'local'">
              <WoodButton variant="ghost" size="sm" @click="openHarvest">
                <AppIcon name="user" :size="14" />
                从账号收录
              </WoodButton>
              <WoodButton variant="ghost" size="sm" @click="playerOpen = true">
                <AppIcon name="globe" :size="14" />
                按玩家名抓取
              </WoodButton>
              <WoodButton variant="primary" size="sm" @click="importOpen = true">
                <AppIcon name="plus" :size="14" />
                导入 PNG
              </WoodButton>
            </template>
          </div>
        </header>

        <section v-show="tab === 'local'" class="lib card">
          <div v-if="loading" class="lib__empty">
            <span class="muted small">正在读取皮肤库…</span>
          </div>
          <div v-else-if="error" class="lib__empty">
            <span class="muted small">{{ error }}</span>
            <WoodButton variant="ghost" size="sm" @click="loadLibrary">重试</WoodButton>
          </div>
          <div v-else-if="!items.length" class="lib__empty">
            <span class="muted small">皮肤库还是空的，导入 PNG 或从账号收录一件吧</span>
          </div>
          <div v-else-if="!filteredItems.length" class="lib__empty">
            <span class="muted small">没有匹配「{{ keyword }}」的皮肤</span>
          </div>

          <div v-else class="skin-grid">
            <article
              v-for="item in filteredItems"
              :key="item.id"
              class="skin"
              :class="{ 'skin--active': previewId === item.id }"
              @click="selectItem(item)"
            >
              <span v-if="isCurrent(item)" class="skin__badge">使用中</span>
              <div class="skin__preview">
                <img v-if="portraits[item.id]" :src="portraits[item.id]" alt="" />
              </div>
              <div class="skin__meta">
                <strong class="skin__name">{{ item.name }}</strong>
                <span class="skin__tags">
                  {{ item.model === "slim" ? "纤细" : "经典" }} · {{ sourceLabel(item.source) }}
                </span>
              </div>
              <footer class="skin__foot">
                <WoodButton variant="primary" size="sm" @click.stop="openApply(item)">
                  应用
                </WoodButton>
                <IconButton
                  icon="settings"
                  label="重命名"
                  :size="26"
                  @click.stop="openRename(item)"
                />
                <IconButton
                  icon="trash"
                  label="删除"
                  :size="26"
                  variant="danger-quiet"
                  @click.stop="deleteTarget = item"
                />
              </footer>
            </article>

            <button class="skin skin--add" @click="importOpen = true">
              <span class="skin--add__plus">+</span>
              <span class="muted small">添加皮肤</span>
            </button>
          </div>
        </section>

        <section v-show="tab === 'littleskin'" class="lib card">
          <LittleSkinPanel :keyword="keyword" />
        </section>

        <section v-show="tab === 'capes'" class="lib card">
          <header class="cape-head">
            <span class="muted small">按账号查看可用的披风</span>
            <WoodSelect
              v-if="capeAccountOptions.length"
              v-model="capeAccount"
              :options="capeAccountOptions"
            />
          </header>

      <div v-if="!capeAccountOptions.length" class="lib__empty">
        <span class="muted small">登录微软或第三方账号后可以查看披风</span>
      </div>
      <div v-else-if="capesLoading" class="lib__empty">
        <span class="muted small">正在读取披风…</span>
      </div>
      <div v-else-if="capesError" class="lib__empty">
        <span class="muted small">{{ capesError }}</span>
        <WoodButton variant="ghost" size="sm" @click="loadCapes">重试</WoodButton>
      </div>
      <div v-else-if="!capes.length" class="lib__empty">
        <span class="muted small">
          {{
            capeAccountView?.kind === "authlib"
              ? "这个角色还没有在皮肤站设置披风"
              : "这个账号没有可用的披风"
          }}
        </span>
      </div>

      <div v-else class="cape-grid">
        <article
          v-for="cape in capes"
          :key="cape.id"
          class="cape"
        >
          <span v-if="cape.active" class="cape__badge">使用中</span>
          <div class="cape__preview">
            <div
              class="cape-front"
              :style="{ backgroundImage: `url(${cape.url})` }"
              role="img"
              :aria-label="cape.alias"
            />
          </div>
          <div class="cape__meta">
            <strong class="cape__name">{{ cape.alias }}</strong>
            <span class="cape__state">{{ cape.active ? "使用中" : "未使用" }}</span>
          </div>
          <footer class="cape__foot">
            <WoodButton
              v-if="capeAccountView?.kind === 'authlib'"
              variant="soft"
              size="sm"
              @click="openSkinSite"
            >
              去皮肤站管理
            </WoodButton>
            <WoodButton
              v-else
              :variant="cape.active ? 'ghost' : 'primary'"
              size="sm"
              :disabled="capeBusy === cape.id"
              @click="toggleCape(cape)"
            >
              {{ capeBusy === cape.id ? "处理中…" : cape.active ? "卸下" : "启用" }}
            </WoodButton>
          </footer>
        </article>
      </div>
        </section>
      </section>
    </div>

    <WoodModal v-if="importOpen" title="导入皮肤 PNG" :width="440" @close="importOpen = false">
      <div class="form">
        <label class="field">
          <span class="field__label">名称</span>
          <WoodInput v-model="importName" placeholder="给皮肤起个名字" />
        </label>
        <label class="field">
          <span class="field__label">模型</span>
          <div class="seg">
            <button
              class="seg__btn"
              :class="{ 'seg__btn--active': importModel === 'classic' }"
              @click="importModel = 'classic'"
            >
              经典
            </button>
            <button
              class="seg__btn"
              :class="{ 'seg__btn--active': importModel === 'slim' }"
              @click="importModel = 'slim'"
            >
              纤细
            </button>
          </div>
        </label>
        <div class="field">
          <span class="field__label">皮肤文件</span>
          <div class="file-row">
            <span class="file-row__path mono">{{ importPath || "未选择文件" }}</span>
            <WoodButton variant="ghost" size="sm" @click="pickImportFile">选择文件</WoodButton>
          </div>
        </div>
      </div>
      <template #footer>
        <WoodButton variant="ghost" size="sm" @click="importOpen = false">取消</WoodButton>
        <WoodButton variant="primary" size="sm" :disabled="importing" @click="confirmImport">
          {{ importing ? "入库中…" : "确认入库" }}
        </WoodButton>
      </template>
    </WoodModal>

    <WoodModal v-if="harvestOpen" title="从账号收录皮肤" :width="440" @close="harvestOpen = false">
      <div class="form">
        <label class="field">
          <span class="field__label">账号</span>
          <WoodSelect v-model="harvestAccount" :options="accountOptions" />
        </label>
        <label class="field">
          <span class="field__label">名称</span>
          <WoodInput v-model="harvestName" placeholder="给皮肤起个名字" />
        </label>
        <p class="muted small">会把该账号当前使用的皮肤复制一份到皮肤库。</p>
      </div>
      <template #footer>
        <WoodButton variant="ghost" size="sm" @click="harvestOpen = false">取消</WoodButton>
        <WoodButton variant="primary" size="sm" :disabled="harvesting" @click="confirmHarvest">
          {{ harvesting ? "收录中…" : "确认收录" }}
        </WoodButton>
      </template>
    </WoodModal>

    <WoodModal v-if="playerOpen" title="按玩家名抓取皮肤" :width="440" @close="playerOpen = false">
      <div class="form">
        <label class="field">
          <span class="field__label">玩家名</span>
          <WoodInput v-model="playerName" placeholder="例如 Notch" @keyup.enter="confirmPlayer" />
        </label>
        <p class="muted small">
          通过 Mojang 官方接口抓取该正版玩家当前使用的皮肤，入库后会用玩家名命名。
        </p>
      </div>
      <template #footer>
        <WoodButton variant="ghost" size="sm" @click="playerOpen = false">取消</WoodButton>
        <WoodButton
          variant="primary"
          size="sm"
          :disabled="playerFetching || !playerName.trim()"
          @click="confirmPlayer"
        >
          {{ playerFetching ? "抓取中…" : "抓取入库" }}
        </WoodButton>
      </template>
    </WoodModal>

    <WoodModal
      v-if="applyTarget"
      :title="`应用皮肤「${applyTarget.name}」`"
      :width="440"
      @close="applyTarget = null"
    >
      <div class="form">
        <label class="field">
          <span class="field__label">目标账号</span>
          <WoodSelect v-model="applyAccount" :options="accountOptions" />
        </label>
        <label v-if="applyAccountView?.kind === 'microsoft'" class="toggle-row">
          <WoodToggle v-model="applyUpload" />
          <span>同步上传 Mojang（替换该账号的官方皮肤，游戏内生效）</span>
        </label>
        <p v-else-if="applyAccountView?.kind === 'authlib'" class="muted small">
          第三方账号的游戏内皮肤由皮肤站下发，启动器没有上传权限。可以先去皮肤站网页上传这张皮肤，
          再用「LittleSkin 衣柜」把它应用到角色。
        </p>
        <WoodButton
          v-if="applyAccountView?.kind === 'authlib'"
          variant="ghost"
          size="sm"
          @click="openAccountSite(applyAccount)"
        >
          <AppIcon name="external" :size="13" />
          打开皮肤站
        </WoodButton>
        <p v-else class="muted small">
          离线账号的皮肤只用于启动器预览与导出，不会改变游戏内显示。
        </p>
      </div>
      <template #footer>
        <WoodButton variant="ghost" size="sm" @click="applyTarget = null">取消</WoodButton>
        <WoodButton variant="primary" size="sm" :disabled="applying" @click="confirmApply">
          {{ applying ? "应用中…" : "确认应用" }}
        </WoodButton>
      </template>
    </WoodModal>

    <WoodModal
      v-if="renameTarget"
      title="重命名皮肤"
      :width="400"
      @close="renameTarget = null"
    >
      <div class="form">
        <label class="field">
          <span class="field__label">新名称</span>
          <WoodInput v-model="renameName" placeholder="皮肤名称" />
        </label>
      </div>
      <template #footer>
        <WoodButton variant="ghost" size="sm" @click="renameTarget = null">取消</WoodButton>
        <WoodButton variant="primary" size="sm" :disabled="renaming" @click="confirmRename">
          {{ renaming ? "保存中…" : "保存" }}
        </WoodButton>
      </template>
    </WoodModal>

    <WoodModal
      v-if="deleteTarget"
      title="删除皮肤"
      :width="400"
      @close="deleteTarget = null"
    >
      <p class="muted small">
        确定要从皮肤库删除「{{ deleteTarget.name }}」吗？此操作不可恢复。
      </p>
      <template #footer>
        <WoodButton variant="ghost" size="sm" @click="deleteTarget = null">取消</WoodButton>
        <WoodButton variant="danger" size="sm" :disabled="deleting" @click="confirmDelete">
          {{ deleting ? "删除中…" : "确认删除" }}
        </WoodButton>
      </template>
    </WoodModal>
  </div>
</template>

<style scoped>
.skins {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

/* 左预览 + 右卡片库 */
.skin-stage {
  display: grid;
  grid-template-columns: 318px minmax(0, 1fr);
  gap: 14px;
  align-items: start;
}

@media (max-width: 1080px) {
  .skin-stage {
    grid-template-columns: 258px minmax(0, 1fr);
  }
}

.preview {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 12px;
}

.preview__viewport {
  display: grid;
  place-items: center;
  border: 1px solid var(--line);
  border-radius: 2px;
  background: radial-gradient(
      circle at 50% 38%,
      rgba(var(--ambient-a), 0.16),
      transparent 68%
    ),
    linear-gradient(180deg, var(--panel-2), var(--panel));
  overflow: hidden;
}

.preview__seg {
  display: flex;
  align-self: center;
  gap: 3px;
  padding: 3px;
  border: 1px solid var(--line);
  border-radius: 3px;
  background: var(--panel-2);
}

.preview__seg button {
  height: 24px;
  padding: 0 16px;
  border-radius: 2px;
  font-size: 11.5px;
  color: var(--ink-soft);
  transition: background var(--dur-1) ease, color var(--dur-1) ease;
}

.preview__seg-btn--active {
  background: var(--panel);
  color: var(--walnut-deep);
  font-weight: 700;
  box-shadow: 0 1px 2px rgba(43, 33, 24, 0.12);
}

.preview__tip {
  margin: 0;
  text-align: center;
  font-size: 10.5px;
  color: var(--muted);
}

.preview__meta {
  display: flex;
  flex-direction: column;
  gap: 2px;
  align-items: center;
  text-align: center;
}

.preview__meta strong {
  font-size: 13px;
  color: var(--walnut-deep);
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.stage-main {
  display: flex;
  flex-direction: column;
  gap: 10px;
  min-width: 0;
}

.stage-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  flex-wrap: wrap;
}

/* 页签沿用项目里的分段按键：凹槽容器 + 选中压入，和导入弹窗的「经典/纤细」同一套 */
.tabs {
  display: flex;
  gap: 2px;
  padding: 3px;
  border: 1px solid var(--line);
  border-radius: 2px;
  background: var(--paper-2);
}

.tabs__btn {
  --face: var(--panel-2);
  height: 28px;
  padding: 0 14px;
  border-radius: 2px;
  border-style: solid;
  border-width: 1px 1px 2px;
  border-color: transparent;
  font-size: 12px;
  font-weight: 600;
  color: var(--ink-soft);
  transition: background 150ms ease, color 150ms ease,
    border-color 150ms ease, transform 100ms ease, box-shadow 100ms ease;
}

.tabs__btn:not(.tabs__btn--active):hover {
  color: var(--walnut-deep);
  background: var(--hover);
  transform: translateY(-1px);
  border-top-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-left-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-right-color: color-mix(in srgb, var(--face) 42%, #000);
  border-bottom-color: color-mix(in srgb, var(--face) 42%, #000);
}

.tabs__btn--active {
  background: var(--panel-2);
  color: var(--walnut-deep);
  font-weight: 700;
  transform: translateY(1px);
  border-top-color: color-mix(in srgb, var(--face) 42%, #000);
  border-left-color: color-mix(in srgb, var(--face) 42%, #000);
  border-right-color: color-mix(in srgb, var(--face) 34%, #fff);
  border-bottom-color: color-mix(in srgb, var(--face) 34%, #fff);
  border-bottom-width: 1px;
  box-shadow: inset 0 2px 3px rgba(0, 0, 0, 0.14);
}

.tabs__count {
  margin-left: 5px;
  font-size: 10px;
  font-weight: 500;
  color: var(--muted);
}

.stage-tools {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}

.search {
  display: flex;
  align-items: center;
  gap: 6px;
  height: 30px;
  padding: 0 10px;
  border: 1px solid var(--line-strong);
  border-radius: 2px;
  background: var(--panel);
  color: var(--muted);
}

.search input {
  width: 132px;
  border: none;
  outline: none;
  background: transparent;
  font-size: 12px;
  color: var(--ink);
}

.cape-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 10px 14px;
  border-bottom: 1px solid var(--line);
}

.lib {
  display: flex;
  flex-direction: column;
  border-radius: 3px;
  overflow: hidden;
}

.lib__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
  padding: 10px 14px;
  border-bottom: 1px solid var(--line);
}

.lib__title {
  display: flex;
  align-items: baseline;
  gap: 8px;
  min-width: 0;
  font-size: 13px;
  font-weight: 700;
  color: var(--walnut-deep);
}

.lib__count {
  font-size: 10.5px;
  font-weight: 500;
  color: var(--muted);
}

.lib__actions {
  display: flex;
  gap: 6px;
}

.lib__empty {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 12px;
  padding: 36px 12px;
}

.skin-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(132px, 1fr));
  gap: 10px;
  padding: 12px;
}

.skin {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 8px;
  border: 1px solid var(--line);
  border-radius: 2px;
  background: var(--panel-2);
  cursor: pointer;
  transition: transform var(--dur-1) ease, border-color var(--dur-1) ease,
    background var(--dur-1) ease;
}

.skin:hover {
  transform: translateY(-1px);
  border-color: var(--line-strong);
  background: var(--panel);
}

.skin--active {
  border-color: var(--moss);
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--moss) 42%, transparent);
}

.skin__badge {
  position: absolute;
  top: 8px;
  right: 8px;
  z-index: 1;
  padding: 1px 6px;
  border-radius: 2px;
  background: var(--moss);
  color: #fffdf6;
  font-size: 10px;
}

.skin__preview {
  display: grid;
  place-items: center;
  aspect-ratio: 1;
  border-radius: 2px;
  background: rgba(var(--glass-rgb), 0.42);
  overflow: hidden;
}

.skin__preview img {
  width: 100%;
  height: 100%;
  object-fit: contain;
  image-rendering: pixelated;
}

/* 最后一格是「添加皮肤」 */
.skin--add {
  align-items: center;
  justify-content: center;
  gap: 8px;
  min-height: 186px;
  border-style: dashed;
  border-color: var(--line-strong);
  background: transparent;
}

.skin--add__plus {
  font-size: 26px;
  line-height: 1;
  color: var(--muted);
}

.skin__meta {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 0 2px;
}

.skin__name {
  font-size: 12.5px;
  font-weight: 700;
  color: var(--walnut-deep);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.skin__tags {
  font-size: 10px;
  color: var(--muted);
}

.skin__foot {
  display: flex;
  align-items: center;
  gap: 4px;
  margin-top: auto;
  padding-top: 6px;
  border-top: 1px solid var(--line);
}

.skin__foot .btn {
  flex: 1;
}

.cape-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(132px, 1fr));
  gap: 10px;
  padding: 12px;
}

.cape {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 8px;
  border: 1px solid var(--line);
  border-radius: 2px;
  background: var(--panel-2);
  transition: transform var(--dur-1) ease, border-color var(--dur-1) ease,
    background var(--dur-1) ease;
}

.cape:hover {
  transform: translateY(-1px);
  border-color: var(--line-strong);
  background: var(--panel);
}

.cape__badge {
  position: absolute;
  top: 8px;
  right: 8px;
  z-index: 1;
  padding: 1px 6px;
  border-radius: 2px;
  background: var(--moss);
  color: #fffdf6;
  font-size: 10px;
}

.cape__preview {
  display: grid;
  place-items: center;
  height: 150px;
  border-radius: 2px;
  background: rgba(var(--glass-rgb), 0.42);
  overflow: hidden;
}

/* 披风纹理是 64x32 的拼接图，只裁外侧面（x=1..11, y=1..17）放大展示，
   避免露出内侧与侧边拼接 */
.cape-front {
  width: 70px;
  height: 112px;
  background-size: 448px 224px;
  background-position: -7px -7px;
  background-repeat: no-repeat;
  image-rendering: pixelated;
}

.cape__meta {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
  padding: 0 2px;
}

.cape__name {
  font-size: 12.5px;
  font-weight: 700;
  color: var(--walnut-deep);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cape__state {
  font-size: 10px;
  color: var(--muted);
}

.cape__foot {
  display: flex;
  align-items: center;
  margin-top: auto;
  padding-top: 6px;
  border-top: 1px solid var(--line);
}

.cape__foot .btn {
  flex: 1;
}

.form {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.field__label {
  font-size: 12px;
  font-weight: 600;
  color: var(--ink-soft);
}

.seg {
  display: flex;
  gap: 2px;
  padding: 3px;
  border-radius: 2px;
  border: 1px solid var(--line);
  background: var(--paper-2);
  align-self: flex-start;
}

/* 分段按键：未选中是平面，悬停浮起带浮雕边，选中压进凹槽 */
.seg__btn {
  --face: var(--panel-2);
  height: 28px;
  padding: 0 14px;
  border-radius: 2px;
  border-style: solid;
  border-width: 1px 1px 2px;
  border-color: transparent;
  font-size: 12px;
  font-weight: 600;
  color: var(--ink-soft);
  transition: background 150ms ease, color 150ms ease,
    border-color 150ms ease, transform 100ms ease, box-shadow 100ms ease;
}

.seg__btn:not(.seg__btn--active):hover {
  color: var(--walnut-deep);
  background: var(--hover);
  transform: translateY(-1px);
  border-top-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-left-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-right-color: color-mix(in srgb, var(--face) 42%, #000);
  border-bottom-color: color-mix(in srgb, var(--face) 42%, #000);
}

.seg__btn:active,
.seg__btn--active {
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

.file-row {
  display: flex;
  align-items: center;
  gap: 10px;
}

.file-row__path {
  flex: 1;
  min-width: 0;
  font-size: 11.5px;
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.toggle-row {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 12.5px;
  color: var(--ink-soft);
}
</style>
