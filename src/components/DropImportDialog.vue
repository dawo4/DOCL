<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { api, type DroppedItem, type PackPreview } from "../api";
import AppIcon from "./AppIcon.vue";
import WoodButton from "./WoodButton.vue";
import WoodModal from "./WoodModal.vue";
import WoodInput from "./WoodInput.vue";
import WoodSelect from "./WoodSelect.vue";
import {
  importModpack,
  notify,
  refreshInstanceMetas,
  selectedVersion,
  store,
} from "../store";

const props = defineProps<{ items: DroppedItem[] }>();
const emit = defineEmits<{ (e: "close"): void; (e: "done"): void }>();

const ICONS: Record<string, string> = {
  mods: "grid",
  resourcepacks: "image",
  shaderpacks: "sun",
  datapacks: "database",
  saves: "cube",
  schematics: "layers",
  modpack: "package",
};

const kindOptions = [
  { value: "mods", label: "模组" },
  { value: "resourcepacks", label: "资源包" },
  { value: "shaderpacks", label: "光影包" },
  { value: "datapacks", label: "数据包" },
  { value: "saves", label: "存档" },
  { value: "schematics", label: "投影文件" },
];

const versions = computed(() =>
  store.installed
    .filter((item) => !item.dependency)
    .map((item) => {
      const meta = store.metas[item.id];
      const name = meta?.display_name?.trim();
      return {
        value: item.id,
        label: name && name !== item.id ? `${name}（${item.id}）` : item.id,
      };
    })
);

const target = ref(
  store.instanceId ?? selectedVersion.value?.id ?? versions.value[0]?.value ?? ""
);
const overrides = ref<Record<string, string>>({});
const busy = ref(false);

const content = computed(() => props.items.filter((item) => item.kind !== "modpack"));
const packs = computed(() => props.items.filter((item) => item.kind === "modpack"));
const unknown = computed(() => content.value.filter((item) => item.kind === "unknown"));

/// 拖入的整合包要判断是客户端包还是服务端包，服务端包直接解压到服务端目录
const packMeta = ref<Record<string, PackPreview | null>>({});

watch(
  packs,
  async (list) => {
    for (const pack of list) {
      if (packMeta.value[pack.path] !== undefined) continue;
      try {
        const preview = await api.modpackInspect(pack.path);
        packMeta.value = { ...packMeta.value, [pack.path]: preview };
      } catch {
        packMeta.value = { ...packMeta.value, [pack.path]: null };
      }
    }
  },
  { immediate: true }
);

function isServerPack(item: DroppedItem) {
  return packMeta.value[item.path]?.server_like === true;
}

const clientPacks = computed(() => packs.value.filter((item) => !isServerPack(item)));
const serverPacks = computed(() => packs.value.filter((item) => isServerPack(item)));

/// 整合包实例名称，默认取清单解析出的名字，用户可以改
const packNames = ref<Record<string, string>>({});

watch(
  packMeta,
  (map) => {
    for (const pack of clientPacks.value) {
      if (packNames.value[pack.path] !== undefined) continue;
      const preview = map[pack.path];
      if (!preview) continue;
      packNames.value = { ...packNames.value, [pack.path]: preview.name };
    }
  },
  { immediate: true }
);

/// 未识别的条目要等用户选好分类才能导入
const unresolved = computed(() => unknown.value.filter((item) => !overrides.value[item.path]));
const canInstall = computed(
  () => !!target.value && content.value.length > 0 && unresolved.value.length === 0
);

function kindOf(item: DroppedItem) {
  return item.kind === "unknown" ? overrides.value[item.path] ?? "" : item.kind;
}

async function installContent() {
  if (!canInstall.value || busy.value) return;
  busy.value = true;
  try {
    let count = 0;
    const installed: string[] = [];
    const failed: string[] = [];
    const unresolved: string[] = [];
    for (const item of content.value) {
      const kind = kindOf(item);
      if (!kind) continue;
      if (kind === "mods") {
        // 模组走带前置检测的通道，装完会自动补齐 Fabric API 之类的必需依赖
        const result = await api.instanceImportMods(target.value, [item.path]);
        count += result.imported;
        installed.push(...result.dependencies.installed);
        failed.push(...result.dependencies.failed);
        unresolved.push(...result.dependencies.unresolved);
      } else {
        count += await api.instanceImportFiles(target.value, kind, [item.path]);
      }
    }
    await refreshInstanceMetas();
    store.contentRevision += 1;

    const notes: string[] = [];
    if (installed.length) notes.push(`已自动补齐前置 ${Array.from(new Set(installed)).join("、")}`);
    if (failed.length) notes.push(`前置下载失败：${Array.from(new Set(failed)).join("、")}`);
    if (unresolved.length) notes.push(`未能自动安装的前置：${Array.from(new Set(unresolved)).join("、")}`);
    notify(
      `已导入 ${count} 项到 ${target.value}${notes.length ? `，${notes.join("；")}` : ""}`,
      unresolved.length || failed.length ? "err" : "ok"
    );
    emit("done");
  } catch (err) {
    notify(`导入失败：${String(err)}`, "err");
  } finally {
    busy.value = false;
  }
}

async function importPacks() {
  if (!clientPacks.value.length || busy.value) return;
  busy.value = true;
  try {
    let last: string | null = null;
    for (const pack of clientPacks.value) {
      const name = packNames.value[pack.path]?.trim() || "";
      const result = await importModpack(
        pack.path,
        { parent_version: target.value || null, instance_name: name || null },
        name || pack.file_name
      );
      if (result) last = result.instance_id;
    }
    if (last) emit("done");
  } finally {
    busy.value = false;
  }
}

/// 服务端整合包：解压到 docl-servers，之后在联机页开服
async function importServerPacks() {
  if (!serverPacks.value.length || busy.value) return;
  busy.value = true;
  try {
    const done: string[] = [];
    let needsInstall = false;
    for (const pack of serverPacks.value) {
      const preview = packMeta.value[pack.path];
      store.installingId = `服务端 · ${pack.file_name}`;
      try {
        const result = await api.serverPackImport(
          pack.path,
          preview?.name || pack.file_name,
          preview?.suggested_id || "",
          null,
          null
        );
        done.push(result.name);
        if (result.needs_install) needsInstall = true;
      } finally {
        store.installingId = null;
      }
    }
    if (done.length) {
      notify(
        needsInstall
          ? `${done.join("、")} 已导入；首次开服会自动安装加载器（需下载依赖）`
          : `${done.join("、")} 已就绪，可在联机页一键开服`
      );
      emit("done");
    }
  } catch (err) {
    notify(`导入服务端失败：${String(err)}`, "err");
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <WoodModal title="导入文件" :width="540" @close="emit('close')">
    <p class="hint">已自动识别文件类型，确认后装到所选版本。整合包会作为新实例导入。</p>

    <ul class="drop-list">
      <li
        v-for="item in items"
        :key="item.path"
        class="drop-item"
        :class="{ 'drop-item--pack': item.kind === 'modpack' }"
      >
        <span class="drop-item__icon">
          <AppIcon :name="ICONS[item.kind] ?? 'package'" :size="16" />
        </span>
        <div class="drop-item__text">
          <strong class="drop-item__name" :title="item.file_name">{{ item.file_name }}</strong>
          <span class="drop-item__meta">
            {{ item.label }}<template v-if="item.detail"> · {{ item.detail }}</template>
            <template v-if="isServerPack(item)"> · 服务端整合包</template>
          </span>
          <div
            v-if="item.kind === 'modpack' && packMeta[item.path] && !isServerPack(item)"
            class="pack-name"
          >
            <WoodInput
              v-model="packNames[item.path]"
              :disabled="busy"
              placeholder="整合包名称"
            />
          </div>
        </div>
        <span v-if="isServerPack(item)" class="tag tag--grass">服务端</span>
        <div v-if="item.kind === 'unknown'" class="drop-item__override">
          <WoodSelect
            v-model="overrides[item.path]"
            :options="kindOptions"
          />
        </div>
      </li>
    </ul>

    <div v-if="content.length" class="target">
      <span class="target__label">安装到</span>
      <WoodSelect
        v-model="target"
        :options="versions"
        :disabled="busy"
      />
    </div>

    <p class="supported">可拖入：模组、资源包、光影、数据包、存档、投影文件、整合包（.zip / .mrpack / .jar）</p>

    <template #footer>
      <span class="footnote">
        {{
          serverPacks.length
            ? "服务端包会解压到服务端目录，装好可在联机页开服"
            : clientPacks.length
              ? "客户端整合包将创建新实例"
              : ""
        }}
      </span>
      <div class="actions">
        <WoodButton variant="ghost" :disabled="busy" @click="emit('close')">取消</WoodButton>
        <WoodButton
          v-if="clientPacks.length"
          variant="soft"
          :disabled="busy"
          @click="importPacks"
        >
          {{ busy ? "导入中…" : `作为整合包导入 (${clientPacks.length})` }}
        </WoodButton>
        <WoodButton
          v-if="serverPacks.length"
          variant="soft"
          :disabled="busy"
          @click="importServerPacks"
        >
          {{ busy ? "导入中…" : `导入为服务端 (${serverPacks.length})` }}
        </WoodButton>
        <WoodButton
          v-if="content.length"
          variant="primary"
          :disabled="busy || !canInstall"
          @click="installContent"
        >
          {{ busy ? "导入中…" : `导入到所选版本 (${content.length})` }}
        </WoodButton>
      </div>
    </template>
  </WoodModal>
</template>

<style scoped>
.hint {
  margin: 0 0 12px;
  font-size: 12.5px;
  color: var(--muted);
  line-height: 1.6;
}

.drop-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin: 0 0 14px;
  padding: 0;
  max-height: 280px;
  overflow-y: auto;
}

.drop-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  border-radius: 0;
  border: 1px solid var(--line);
  background: var(--panel-2);
  /* 列表行做成浅凹槽 */
  box-shadow: inset 0 1px 2px rgba(60, 40, 20, 0.06);
}

.drop-item--pack {
  border-color: color-mix(in srgb, var(--moss) 42%, var(--line-strong));
  background: var(--moss-soft);
}

.drop-item__icon {
  display: grid;
  place-items: center;
  width: 30px;
  height: 30px;
  border-radius: 0;
  border-style: solid;
  border-width: 1px;
  border-top-color: color-mix(in srgb, var(--line-strong) 62%, #000);
  border-left-color: color-mix(in srgb, var(--line-strong) 62%, #000);
  border-right-color: color-mix(in srgb, var(--line-strong) 38%, #fff);
  border-bottom-color: color-mix(in srgb, var(--line-strong) 38%, #fff);
  background: var(--panel);
  color: var(--moss);
  flex-shrink: 0;
}

.drop-item__text {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
  flex: 1;
}

.drop-item__name {
  font-size: 13px;
  color: var(--walnut-deep);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.drop-item__meta {
  font-size: 11px;
  color: var(--muted);
}

.drop-item__override {
  width: 130px;
  flex-shrink: 0;
}

.pack-name {
  margin-top: 5px;
}

.pack-name :deep(.input) {
  height: 30px;
  padding: 0 10px;
  font-size: 12.5px;
}

.target {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 10px;
}

.target__label {
  flex-shrink: 0;
  font-size: 12px;
  font-weight: 700;
  color: var(--ink-soft);
}

.target :deep(.select) {
  flex: 1;
  min-width: 0;
}

.supported {
  margin: 0;
  font-size: 11px;
  color: var(--muted);
  line-height: 1.6;
}

.footnote {
  flex: 1;
  font-size: 11px;
  color: var(--muted);
}

.actions {
  display: flex;
  align-items: center;
  gap: 8px;
}
</style>
