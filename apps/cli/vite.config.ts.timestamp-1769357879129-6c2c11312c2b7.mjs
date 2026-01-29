// vite.config.ts
import { defineConfig } from "file:///Users/khadim/dev/spooky/node_modules/.pnpm/vite@5.4.20_@types+node@20.19.22_lightningcss@1.30.1/node_modules/vite/dist/node/index.js";
import { resolve } from "path";
import dts from "file:///Users/khadim/dev/spooky/node_modules/.pnpm/vite-plugin-dts@3.9.1_@types+node@20.19.22_rollup@4.52.4_typescript@5.9.3_vite@5.4.20_@types+_4zlyq66do3okskutaxkm4ts3ha/node_modules/vite-plugin-dts/dist/index.mjs";
var __vite_injected_original_dirname = "/Users/khadim/dev/spooky/apps/cli";
var vite_config_default = defineConfig({
  plugins: [
    dts({
      insertTypesEntry: true,
      tsconfigPath: "./tsconfig.json"
    })
  ],
  build: {
    lib: {
      entry: {
        syncgen: resolve(__vite_injected_original_dirname, "src/index.ts"),
        cli: resolve(__vite_injected_original_dirname, "src/cli.ts")
      },
      name: "syncgen",
      formats: ["es", "cjs"],
      fileName: (format, entryName) => {
        if (format === "es") {
          return `${entryName}.js`;
        }
        return `${entryName}.cjs`;
      }
    },
    rollupOptions: {
      external: ["commander", "child_process", "url", "path", "fs", "os", "util"],
      output: {
        preserveModules: false,
        banner: (chunk) => {
          if (chunk.name === "cli") {
            return "#!/usr/bin/env node";
          }
          return "";
        }
      }
    }
  },
  resolve: {
    alias: {
      "@": resolve(__vite_injected_original_dirname, "./src")
    }
  }
});
export {
  vite_config_default as default
};
//# sourceMappingURL=data:application/json;base64,ewogICJ2ZXJzaW9uIjogMywKICAic291cmNlcyI6IFsidml0ZS5jb25maWcudHMiXSwKICAic291cmNlc0NvbnRlbnQiOiBbImNvbnN0IF9fdml0ZV9pbmplY3RlZF9vcmlnaW5hbF9kaXJuYW1lID0gXCIvVXNlcnMva2hhZGltL2Rldi9zcG9va3kvYXBwcy9jbGlcIjtjb25zdCBfX3ZpdGVfaW5qZWN0ZWRfb3JpZ2luYWxfZmlsZW5hbWUgPSBcIi9Vc2Vycy9raGFkaW0vZGV2L3Nwb29reS9hcHBzL2NsaS92aXRlLmNvbmZpZy50c1wiO2NvbnN0IF9fdml0ZV9pbmplY3RlZF9vcmlnaW5hbF9pbXBvcnRfbWV0YV91cmwgPSBcImZpbGU6Ly8vVXNlcnMva2hhZGltL2Rldi9zcG9va3kvYXBwcy9jbGkvdml0ZS5jb25maWcudHNcIjtpbXBvcnQgeyBkZWZpbmVDb25maWcgfSBmcm9tICd2aXRlJztcbmltcG9ydCB7IHJlc29sdmUgfSBmcm9tICdwYXRoJztcbmltcG9ydCBkdHMgZnJvbSAndml0ZS1wbHVnaW4tZHRzJztcblxuZXhwb3J0IGRlZmF1bHQgZGVmaW5lQ29uZmlnKHtcbiAgcGx1Z2luczogW1xuICAgIGR0cyh7XG4gICAgICBpbnNlcnRUeXBlc0VudHJ5OiB0cnVlLFxuICAgICAgdHNjb25maWdQYXRoOiAnLi90c2NvbmZpZy5qc29uJyxcbiAgICB9KSxcbiAgXSxcbiAgYnVpbGQ6IHtcbiAgICBsaWI6IHtcbiAgICAgIGVudHJ5OiB7XG4gICAgICAgIHN5bmNnZW46IHJlc29sdmUoX19kaXJuYW1lLCAnc3JjL2luZGV4LnRzJyksXG4gICAgICAgIGNsaTogcmVzb2x2ZShfX2Rpcm5hbWUsICdzcmMvY2xpLnRzJyksXG4gICAgICB9LFxuICAgICAgbmFtZTogJ3N5bmNnZW4nLFxuICAgICAgZm9ybWF0czogWydlcycsICdjanMnXSxcbiAgICAgIGZpbGVOYW1lOiAoZm9ybWF0LCBlbnRyeU5hbWUpID0+IHtcbiAgICAgICAgaWYgKGZvcm1hdCA9PT0gJ2VzJykge1xuICAgICAgICAgIHJldHVybiBgJHtlbnRyeU5hbWV9LmpzYDtcbiAgICAgICAgfVxuICAgICAgICByZXR1cm4gYCR7ZW50cnlOYW1lfS5janNgO1xuICAgICAgfSxcbiAgICB9LFxuICAgIHJvbGx1cE9wdGlvbnM6IHtcbiAgICAgIGV4dGVybmFsOiBbJ2NvbW1hbmRlcicsICdjaGlsZF9wcm9jZXNzJywgJ3VybCcsICdwYXRoJywgJ2ZzJywgJ29zJywgJ3V0aWwnXSxcbiAgICAgIG91dHB1dDoge1xuICAgICAgICBwcmVzZXJ2ZU1vZHVsZXM6IGZhbHNlLFxuICAgICAgICBiYW5uZXI6IChjaHVuaykgPT4ge1xuICAgICAgICAgIGlmIChjaHVuay5uYW1lID09PSAnY2xpJykge1xuICAgICAgICAgICAgcmV0dXJuICcjIS91c3IvYmluL2VudiBub2RlJztcbiAgICAgICAgICB9XG4gICAgICAgICAgcmV0dXJuICcnO1xuICAgICAgICB9LFxuICAgICAgfSxcbiAgICB9LFxuICB9LFxuICByZXNvbHZlOiB7XG4gICAgYWxpYXM6IHtcbiAgICAgICdAJzogcmVzb2x2ZShfX2Rpcm5hbWUsICcuL3NyYycpLFxuICAgIH0sXG4gIH0sXG59KTtcbiJdLAogICJtYXBwaW5ncyI6ICI7QUFBcVIsU0FBUyxvQkFBb0I7QUFDbFQsU0FBUyxlQUFlO0FBQ3hCLE9BQU8sU0FBUztBQUZoQixJQUFNLG1DQUFtQztBQUl6QyxJQUFPLHNCQUFRLGFBQWE7QUFBQSxFQUMxQixTQUFTO0FBQUEsSUFDUCxJQUFJO0FBQUEsTUFDRixrQkFBa0I7QUFBQSxNQUNsQixjQUFjO0FBQUEsSUFDaEIsQ0FBQztBQUFBLEVBQ0g7QUFBQSxFQUNBLE9BQU87QUFBQSxJQUNMLEtBQUs7QUFBQSxNQUNILE9BQU87QUFBQSxRQUNMLFNBQVMsUUFBUSxrQ0FBVyxjQUFjO0FBQUEsUUFDMUMsS0FBSyxRQUFRLGtDQUFXLFlBQVk7QUFBQSxNQUN0QztBQUFBLE1BQ0EsTUFBTTtBQUFBLE1BQ04sU0FBUyxDQUFDLE1BQU0sS0FBSztBQUFBLE1BQ3JCLFVBQVUsQ0FBQyxRQUFRLGNBQWM7QUFDL0IsWUFBSSxXQUFXLE1BQU07QUFDbkIsaUJBQU8sR0FBRyxTQUFTO0FBQUEsUUFDckI7QUFDQSxlQUFPLEdBQUcsU0FBUztBQUFBLE1BQ3JCO0FBQUEsSUFDRjtBQUFBLElBQ0EsZUFBZTtBQUFBLE1BQ2IsVUFBVSxDQUFDLGFBQWEsaUJBQWlCLE9BQU8sUUFBUSxNQUFNLE1BQU0sTUFBTTtBQUFBLE1BQzFFLFFBQVE7QUFBQSxRQUNOLGlCQUFpQjtBQUFBLFFBQ2pCLFFBQVEsQ0FBQyxVQUFVO0FBQ2pCLGNBQUksTUFBTSxTQUFTLE9BQU87QUFDeEIsbUJBQU87QUFBQSxVQUNUO0FBQ0EsaUJBQU87QUFBQSxRQUNUO0FBQUEsTUFDRjtBQUFBLElBQ0Y7QUFBQSxFQUNGO0FBQUEsRUFDQSxTQUFTO0FBQUEsSUFDUCxPQUFPO0FBQUEsTUFDTCxLQUFLLFFBQVEsa0NBQVcsT0FBTztBQUFBLElBQ2pDO0FBQUEsRUFDRjtBQUNGLENBQUM7IiwKICAibmFtZXMiOiBbXQp9Cg==
