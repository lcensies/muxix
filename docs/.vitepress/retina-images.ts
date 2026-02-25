import type { Plugin } from "vite";
import fs from "fs";
import path from "path";
import sharp from "sharp";

const IMG_RE = /<img([^>]*?)src="\/([^"]+\.webp)"([^>]*?)>/g;

export function retinaImagesPlugin(publicDir: string): Plugin {
  return {
    name: "vitepress-retina-images",
    enforce: "pre",

    async buildStart() {
      await generateRetina1x(publicDir);
    },

    transform(code, id) {
      if (!id.endsWith(".md")) return null;

      const result = code.replace(IMG_RE, (match, before, src, after) => {
        if (match.includes("srcset=") || match.includes("data-no-retina"))
          return match;
        const name = src.replace(/\.webp$/, "");
        return `<img${before}src="/${src}" srcset="/_1x/${name}.webp 1x, /${src} 2x"${after}>`;
      });

      return result !== code ? result : null;
    },
  };
}

export async function generateRetina1x(dir: string): Promise<void> {
  const files = fs.readdirSync(dir).filter((f) => f.endsWith(".webp"));

  if (files.length === 0) return;

  const oneXDir = path.join(dir, "_1x");
  if (!fs.existsSync(oneXDir)) {
    fs.mkdirSync(oneXDir, { recursive: true });
  }

  await Promise.all(
    files.map(async (file) => {
      const src = path.join(dir, file);
      const dest = path.join(oneXDir, file);

      // Skip if already generated and up to date
      if (fs.existsSync(dest)) {
        const srcStat = fs.statSync(src);
        const destStat = fs.statSync(dest);
        if (destStat.mtimeMs >= srcStat.mtimeMs) return;
      }

      const metadata = await sharp(src).metadata();
      await sharp(src)
        .resize(Math.round(metadata.width! / 2))
        .webp()
        .toFile(dest);
    }),
  );
}
