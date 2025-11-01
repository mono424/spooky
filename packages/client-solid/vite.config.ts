import { defineConfig } from "vite";
import dts from "vite-plugin-dts";

export default defineConfig({
  plugins: [dts({ rollupTypes: true })],
  build: {
    lib: {
      entry: "src/index.ts",
      name: "DbSolid",
      fileName: (format) => `index.${format === "es" ? "mjs" : "js"}`,
      formats: ["es", "cjs"],
    },
    rollupOptions: {
      external: ["surrealdb", "@surrealdb/wasm", "solid-js"],
      output: {
        preserveModules: false,
        globals: {
          surrealdb: "Surreal",
          "@surrealdb/wasm": "SurrealWasm",
        },
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
