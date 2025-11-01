import { defineConfig } from "vite";

export default defineConfig({
  build: {
    lib: {
      entry: "src/index.ts",
      name: "DbSolid",
      fileName: (format) => `index.${format === "es" ? "mjs" : "js"}`,
      formats: ["es", "cjs"],
    },
    rollupOptions: {
      external: ["solid-js"],
      output: {
        preserveModules: false,
      },
    },
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: true,
    minify: false,
    target: "es2020",
  },
  esbuild: {
    loader: "ts",
  },
  resolve: {
    extensions: [".ts", ".js"],
  },
});
