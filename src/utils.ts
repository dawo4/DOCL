export function formatBytes(bytes: number): string {
  if (!bytes || bytes < 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let index = 0;
  while (value >= 1024 && index < units.length - 1) {
    value /= 1024;
    index += 1;
  }
  return `${value >= 100 ? value.toFixed(0) : value.toFixed(1)} ${units[index]}`;
}

export function formatSpeed(bytes: number, elapsedMs: number): string {
  if (elapsedMs <= 0) return "—";
  return `${formatBytes((bytes / elapsedMs) * 1000)}/s`;
}

export function kindLabel(kind: string): string {
  switch (kind) {
    case "release":
      return "正式版";
    case "snapshot":
      return "快照";
    case "old_beta":
      return "远古 Beta";
    case "old_alpha":
      return "远古 Alpha";
    default:
      return kind || "未知";
  }
}

export function formatDate(iso?: string | null): string {
  if (!iso) return "—";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(
    date.getDate()
  ).padStart(2, "0")}`;
}

export function formatDuration(ms: number): string {
  const total = Math.floor(ms / 1000);
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return minutes > 0 ? `${minutes} 分 ${seconds} 秒` : `${seconds} 秒`;
}

/// 窗口最小化 / 被完全遮挡时跳过这一轮轮询：
/// 定时器本身不销毁，恢复可见后由下一次触发自然刷新，避免积压请求。
export function skipWhenHidden<A extends unknown[]>(fn: (...args: A) => void) {
  return (...args: A) => {
    if (document.hidden) return;
    fn(...args);
  };
}

export interface IconPreset {
  value: string;
  label: string;
  color: string;
}

/// 版本图标预设，版本列表与版本设置共用
export const ICON_PRESETS: IconPreset[] = [
  { value: "auto", label: "自动", color: "#8b9d6f" },
  { value: "grass", label: "草方块", color: "#6f8f5a" },
  { value: "diamond", label: "钻石", color: "#5fa7a0" },
  { value: "gold", label: "黄金", color: "#cf9f45" },
  { value: "redstone", label: "红石", color: "#b8543f" },
  { value: "nether", label: "下界", color: "#7c4a3f" },
  { value: "end", label: "末地", color: "#6f5f86" },
];

export function iconColor(value?: string): string {
  return ICON_PRESETS.find((preset) => preset.value === value)?.color ?? "#8b9d6f";
}

/* ---------- 版本分类 ---------- */

/// 官方清单里的愚人节彩蛋版本（kind 都标成 snapshot，按 id 单独归一档）
export const APRIL_FOOLS_IDS: ReadonlySet<string> = new Set([
  "2.0",
  "15w14a",
  "1.RV-Pre1",
  "20w14infinite",
  "3D Shareware v1.34",
]);

export type VersionFilter = "release" | "snapshot" | "legacy" | "april";

export function matchVersionFilter(kind: string, id: string, target: VersionFilter): boolean {
  switch (target) {
    case "release":
      return kind === "release";
    case "snapshot":
      return kind === "snapshot" && !APRIL_FOOLS_IDS.has(id);
    case "april":
      return APRIL_FOOLS_IDS.has(id);
    case "legacy":
      return kind === "old_alpha" || kind === "old_beta";
  }
}
