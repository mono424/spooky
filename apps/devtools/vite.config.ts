import { defineConfig } from "vite";
import { resolve } from "path";
import { copyFileSync, mkdirSync, existsSync, readdirSync, statSync } from "fs";
import { fileURLToPath } from "url";
import { dirname, join } from "path";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

// Plugin to copy manifest.json and icons to dist
const copyAssetsPlugin = () => ({
  name: "copy-assets",
  closeBundle() {
    // Copy manifest.json
    const manifestPath = resolve(__dirname, "manifest.json");
    const distPath = resolve(__dirname, "dist", "manifest.json");
    if (existsSync(manifestPath)) {
      copyFileSync(manifestPath, distPath);
      console.log("✓ Copied manifest.json to dist");
    }

    // Copy icons folder
    const iconsSource = resolve(__dirname, "icons");
    const iconsPublic = resolve(__dirname, "public", "icons");
    const iconsDest = resolve(__dirname, "dist", "icons");

    // Determine which source to use (prefer root icons folder)
    const iconsDir = existsSync(iconsSource) ? iconsSource : iconsPublic;

    if (existsSync(iconsDir)) {
      // Create icons directory in dist if it doesn't exist
      if (!existsSync(iconsDest)) {
        mkdirSync(iconsDest, { recursive: true });
      }

      // Copy all icon files
      const files = readdirSync(iconsDir);
      files.forEach(file => {
        const sourcePath = join(iconsDir, file);
        const destPath = join(iconsDest, file);

        // Only copy files, not directories
        if (statSync(sourcePath).isFile()) {
          copyFileSync(sourcePath, destPath);
          console.log(`✓ Copied ${file} to dist/icons`);
        }
      });
    } else {
      console.warn("⚠ Icons folder not found");
    }
  },
});

export default defineConfig({
  plugins: [copyAssetsPlugin()],
  build: {
    outDir: "dist",
    rollupOptions: {
      input: {
        devtools: resolve(__dirname, "src/devtools.ts"),
        panel: resolve(__dirname, "src/panel.ts"),
        background: resolve(__dirname, "src/background.ts"),
        content: resolve(__dirname, "src/content.ts"),
        "page-script": resolve(__dirname, "src/page-script.ts"),
      },
      output: {
        entryFileNames: "[name].js",
        format: "es",
      },
    },
    minify: false,
    sourcemap: true,
  },
  publicDir: "public",
});
