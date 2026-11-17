import { describe, expect, it } from "vitest";
import { bresenham, floodFill, hexToRgba, rgbaToHex, type Rgba } from "./paint";

/// 只实现 floodFill 用到的 canvas 接口，避免为几个纯函数引入 jsdom
function fakeContext(width: number, height: number, pixels?: Rgba[]) {
  const data = new Uint8ClampedArray(width * height * 4);
  for (let index = 0; index < width * height; index += 1) {
    const pixel = pixels?.[index] ?? [0, 0, 0, 0];
    data.set(pixel, index * 4);
  }
  return {
    canvas: { width, height },
    getImageData: (x: number, y: number, w: number, h: number) => {
      const out = new Uint8ClampedArray(w * h * 4);
      for (let row = 0; row < h; row += 1) {
        const start = ((y + row) * width + x) * 4;
        out.set(data.subarray(start, start + w * 4), row * w * 4);
      }
      return { data: out, width: w, height: h };
    },
    putImageData: (
      image: { data: Uint8ClampedArray; width: number; height: number },
      x: number,
      y: number
    ) => {
      for (let row = 0; row < image.height; row += 1) {
        const dest = ((y + row) * width + x) * 4;
        const start = row * image.width * 4;
        data.set(image.data.subarray(start, start + image.width * 4), dest);
      }
    },
    pixel: (x: number, y: number): Rgba => {
      const offset = (y * width + x) * 4;
      return [data[offset], data[offset + 1], data[offset + 2], data[offset + 3]];
    },
  };
}

describe("hexToRgba / rgbaToHex", () => {
  it("支持三位与六位十六进制", () => {
    expect(hexToRgba("#f80")).toEqual([255, 136, 0, 255]);
    expect(hexToRgba("#336699")).toEqual([51, 102, 153, 255]);
  });

  it("非法输入回退为黑色", () => {
    expect(hexToRgba("oops")).toEqual([0, 0, 0, 255]);
  });

  it("转换回十六进制时夹紧取值范围", () => {
    expect(rgbaToHex(300, -5, 16)).toBe("#ff0010");
  });
});

describe("bresenham", () => {
  it("水平线包含两个端点", () => {
    expect(bresenham(0, 0, 3, 0)).toEqual([
      [0, 0],
      [1, 0],
      [2, 0],
      [3, 0],
    ]);
  });

  it("单点只返回一个坐标", () => {
    expect(bresenham(2, 2, 2, 2)).toEqual([[2, 2]]);
  });
});

describe("floodFill", () => {
  it("只填充连通的同色区域", () => {
    const red: Rgba = [255, 0, 0, 255];
    // 3x3 全透明，中间一格是黑的不透明像素
    const pixels = Array.from({ length: 9 }, (): Rgba => [0, 0, 0, 0]);
    pixels[4] = [0, 0, 0, 255];
    const ctx = fakeContext(3, 3, pixels);
    floodFill(ctx as unknown as CanvasRenderingContext2D, 0, 0, red);
    expect(ctx.pixel(0, 0)).toEqual(red);
    expect(ctx.pixel(2, 2)).toEqual(red);
    expect(ctx.pixel(1, 1)).toEqual([0, 0, 0, 255]);
  });

  it("遵守 bounds，不会漏到别的部位", () => {
    const blue: Rgba = [0, 0, 255, 255];
    const ctx = fakeContext(4, 4);
    floodFill(ctx as unknown as CanvasRenderingContext2D, 0, 0, blue, [0, 0, 2, 2]);
    expect(ctx.pixel(1, 1)).toEqual(blue);
    expect(ctx.pixel(2, 2)).toEqual([0, 0, 0, 0]);
  });

  it("起点越界时什么都不做", () => {
    const ctx = fakeContext(2, 2);
    floodFill(ctx as unknown as CanvasRenderingContext2D, 9, 9, [1, 2, 3, 255]);
    expect(ctx.pixel(0, 0)).toEqual([0, 0, 0, 0]);
  });
});
