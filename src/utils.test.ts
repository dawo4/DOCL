import { describe, expect, it } from "vitest";
import {
  formatBytes,
  formatDuration,
  formatSpeed,
  isSupportedLoader,
  loaderLabel,
  matchVersionFilter,
} from "./utils";

describe("formatBytes", () => {
  it("处理空值与负数", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(-5)).toBe("0 B");
  });

  it("按 1024 进制换算", () => {
    expect(formatBytes(1024)).toBe("1.0 KB");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(200 * 1024)).toBe("200 KB");
  });
});

describe("formatSpeed", () => {
  it("没有耗时时不显示速度", () => {
    expect(formatSpeed(1024, 0)).toBe("—");
  });

  it("换算成每秒字节数", () => {
    expect(formatSpeed(1024, 1000)).toBe("1.0 KB/s");
  });
});

describe("formatDuration", () => {
  it("不足一分钟只显示秒", () => {
    expect(formatDuration(5000)).toBe("5 秒");
  });

  it("超过一分钟拆成分与秒", () => {
    expect(formatDuration(65000)).toBe("1 分 5 秒");
  });
});

describe("matchVersionFilter", () => {
  it("愚人节版本不混进快照", () => {
    expect(matchVersionFilter("snapshot", "25w14craftmine", "snapshot")).toBe(true);
    expect(matchVersionFilter("snapshot", "15w14a", "snapshot")).toBe(false);
    expect(matchVersionFilter("snapshot", "15w14a", "april")).toBe(true);
  });

  it("远古版本按 alpha/beta 归类", () => {
    expect(matchVersionFilter("old_beta", "b1.7.3", "legacy")).toBe(true);
    expect(matchVersionFilter("release", "1.21.1", "legacy")).toBe(false);
  });
});

describe("loaderLabel", () => {
  it("原版与常见加载器", () => {
    expect(loaderLabel(undefined)).toBe("原版");
    expect(loaderLabel("vanilla")).toBe("原版");
    expect(loaderLabel("forge")).toBe("Forge");
    expect(loaderLabel("neoforge")).toBe("NeoForge");
  });

  it("装不了的加载器带「暂不支持」，不能显示成原版", () => {
    expect(loaderLabel("cleanroom")).toBe("Cleanroom（暂不支持）");
    expect(loaderLabel("liteloader")).toBe("LiteLoader（暂不支持）");
    expect(loaderLabel("unknown")).toBe("未知加载器（暂不支持）");
  });
});

describe("isSupportedLoader", () => {
  it("只有能自动安装的才算支持", () => {
    expect(isSupportedLoader("forge")).toBe(true);
    expect(isSupportedLoader("fabric")).toBe(true);
    expect(isSupportedLoader("cleanroom")).toBe(false);
    expect(isSupportedLoader("unknown")).toBe(false);
  });
});
