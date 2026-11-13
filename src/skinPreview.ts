/// 把 64×64 皮肤图拼成 16×32 的正面人形立绘，用于皮肤库卡片缩略图。
/// 只读取正面的六块区域 + 对应外层，不依赖 WebGL，一张卡片一个 <img> 就够了。

interface Patch {
  sx: number;
  sy: number;
  sw: number;
  sh: number;
  dx: number;
  dy: number;
  dw: number;
  dh: number;
}

function patches(slim: boolean): Patch[] {
  const armWidth = slim ? 3 : 4;
  const armDx = slim ? 1 : 0;
  const armDxRight = slim ? 12 : 12;
  return [
    // 基础层
    { sx: 8, sy: 8, sw: 8, sh: 8, dx: 4, dy: 0, dw: 8, dh: 8 }, // 头
    { sx: 20, sy: 20, sw: 8, sh: 12, dx: 4, dy: 8, dw: 8, dh: 12 }, // 身体
    { sx: 44, sy: 20, sw: armWidth, sh: 12, dx: armDx, dy: 8, dw: armWidth, dh: 12 }, // 右臂
    { sx: 36, sy: 52, sw: armWidth, sh: 12, dx: armDxRight, dy: 8, dw: armWidth, dh: 12 }, // 左臂
    { sx: 4, sy: 20, sw: 4, sh: 12, dx: 4, dy: 20, dw: 4, dh: 12 }, // 右腿
    { sx: 20, sy: 52, sw: 4, sh: 12, dx: 8, dy: 20, dw: 4, dh: 12 }, // 左腿
    // 外层：透明像素会自然透出基础层
    { sx: 40, sy: 8, sw: 8, sh: 8, dx: 4, dy: 0, dw: 8, dh: 8 }, // 帽子
    { sx: 20, sy: 36, sw: 8, sh: 12, dx: 4, dy: 8, dw: 8, dh: 12 }, // 外套
    { sx: 44, sy: 36, sw: armWidth, sh: 12, dx: armDx, dy: 8, dw: armWidth, dh: 12 }, // 右袖
    { sx: 52, sy: 52, sw: armWidth, sh: 12, dx: armDxRight, dy: 8, dw: armWidth, dh: 12 }, // 左袖
    { sx: 0, sy: 36, sw: 4, sh: 12, dx: 4, dy: 20, dw: 4, dh: 12 }, // 右裤
    { sx: 4, sy: 52, sw: 4, sh: 12, dx: 8, dy: 20, dw: 4, dh: 12 }, // 左裤
  ];
}

function loadImage(source: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => reject(new Error("皮肤图读取失败"));
    image.src = source;
  });
}

export async function skinPortrait(source: string, model: string): Promise<string> {
  if (!source) return "";
  const image = await loadImage(source);
  const canvas = document.createElement("canvas");
  canvas.width = 16;
  canvas.height = 32;
  const ctx = canvas.getContext("2d");
  if (!ctx) return "";
  ctx.imageSmoothingEnabled = false;
  for (const patch of patches(model === "slim")) {
    ctx.drawImage(
      image,
      patch.sx,
      patch.sy,
      patch.sw,
      patch.sh,
      patch.dx,
      patch.dy,
      patch.dw,
      patch.dh
    );
  }
  return canvas.toDataURL("image/png");
}
