import { defineConfig } from "vite";
import { resolve } from "path";
import dts from "vite-plugin-dts";

export default defineConfig({
  plugins: [
    dts({
      insertTypesEntry: true,
      tsconfigPath: "./tsconfig.json",
    }),
  ],
  build: {
    lib: {
      entry: {
        syncgen: resolve(__dirname, "src/index.ts"),
        cli: resolve(__dirname, "src/cli.ts"),
      },
      name: "syncgen",
      formats: ["es", "cjs"],
      fileName: (format, entryName) => {
        if (format === "es") {
          return `${entryName}.js`;
        }
        return `${entryName}.cjs`;
      },
    },
    rollupOptions: {
      external: ["commander", "child_process", "url", "path", "fs", "os"],
      output: {
        preserveModules: false,
        banner: (chunk) => {
          if (chunk.name === "cli") {
            return "#!/usr/bin/env node";
          }
          return "";
        },
      },
    },
  },
  resolve: {
    alias: {
      "@": resolve(__dirname, "./src"),
    },
  },
});
