// vite.config.ts
import { defineConfig } from "file:///Users/khadim/dev/spooky/node_modules/.pnpm/vite@5.4.20_@types+node@20.19.22_lightningcss@1.30.1/node_modules/vite/dist/node/index.js";
import dts from "file:///Users/khadim/dev/spooky/node_modules/.pnpm/vite-plugin-dts@4.5.4_@types+node@20.19.22_rollup@4.52.4_typescript@5.9.3_vite@5.4.20_@types+_4i77tizfn53bgzkniabiljknxy/node_modules/vite-plugin-dts/dist/index.mjs";
var vite_config_default = defineConfig({
  plugins: [dts({ rollupTypes: true })],
  build: {
    lib: {
      entry: "src/index.ts",
      name: "DbSolid",
      fileName: (format) => `index.${format === "es" ? "mjs" : "cjs"}`,
      formats: ["es", "cjs"]
    },
    rollupOptions: {
      external: [
        "surrealdb",
        "@surrealdb/wasm",
        "solid-js",
        "@spooky/core",
        "@spooky/query-builder",
        "effect",
        "valtio"
      ],
      output: {
        preserveModules: false,
        globals: {
          surrealdb: "Surreal",
          "@surrealdb/wasm": "SurrealWasm"
        }
      }
    },
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: true,
    minify: false,
    target: "es2020"
  },
  esbuild: {
    loader: "ts"
  },
  resolve: {
    extensions: [".ts", ".js"]
  }
});
export {
  vite_config_default as default
};
//# sourceMappingURL=data:application/json;base64,ewogICJ2ZXJzaW9uIjogMywKICAic291cmNlcyI6IFsidml0ZS5jb25maWcudHMiXSwKICAic291cmNlc0NvbnRlbnQiOiBbImNvbnN0IF9fdml0ZV9pbmplY3RlZF9vcmlnaW5hbF9kaXJuYW1lID0gXCIvVXNlcnMva2hhZGltL2Rldi9zcG9va3kvcGFja2FnZXMvY2xpZW50LXNvbGlkXCI7Y29uc3QgX192aXRlX2luamVjdGVkX29yaWdpbmFsX2ZpbGVuYW1lID0gXCIvVXNlcnMva2hhZGltL2Rldi9zcG9va3kvcGFja2FnZXMvY2xpZW50LXNvbGlkL3ZpdGUuY29uZmlnLnRzXCI7Y29uc3QgX192aXRlX2luamVjdGVkX29yaWdpbmFsX2ltcG9ydF9tZXRhX3VybCA9IFwiZmlsZTovLy9Vc2Vycy9raGFkaW0vZGV2L3Nwb29reS9wYWNrYWdlcy9jbGllbnQtc29saWQvdml0ZS5jb25maWcudHNcIjtpbXBvcnQgeyBkZWZpbmVDb25maWcgfSBmcm9tICd2aXRlJztcbmltcG9ydCBkdHMgZnJvbSAndml0ZS1wbHVnaW4tZHRzJztcblxuZXhwb3J0IGRlZmF1bHQgZGVmaW5lQ29uZmlnKHtcbiAgcGx1Z2luczogW2R0cyh7IHJvbGx1cFR5cGVzOiB0cnVlIH0pXSxcbiAgYnVpbGQ6IHtcbiAgICBsaWI6IHtcbiAgICAgIGVudHJ5OiAnc3JjL2luZGV4LnRzJyxcbiAgICAgIG5hbWU6ICdEYlNvbGlkJyxcbiAgICAgIGZpbGVOYW1lOiAoZm9ybWF0KSA9PiBgaW5kZXguJHtmb3JtYXQgPT09ICdlcycgPyAnbWpzJyA6ICdjanMnfWAsXG4gICAgICBmb3JtYXRzOiBbJ2VzJywgJ2NqcyddLFxuICAgIH0sXG4gICAgcm9sbHVwT3B0aW9uczoge1xuICAgICAgZXh0ZXJuYWw6IFtcbiAgICAgICAgJ3N1cnJlYWxkYicsXG4gICAgICAgICdAc3VycmVhbGRiL3dhc20nLFxuICAgICAgICAnc29saWQtanMnLFxuICAgICAgICAnQHNwb29reS9jb3JlJyxcbiAgICAgICAgJ0BzcG9va3kvcXVlcnktYnVpbGRlcicsXG4gICAgICAgICdlZmZlY3QnLFxuICAgICAgICAndmFsdGlvJyxcbiAgICAgIF0sXG4gICAgICBvdXRwdXQ6IHtcbiAgICAgICAgcHJlc2VydmVNb2R1bGVzOiBmYWxzZSxcbiAgICAgICAgZ2xvYmFsczoge1xuICAgICAgICAgIHN1cnJlYWxkYjogJ1N1cnJlYWwnLFxuICAgICAgICAgICdAc3VycmVhbGRiL3dhc20nOiAnU3VycmVhbFdhc20nLFxuICAgICAgICB9LFxuICAgICAgfSxcbiAgICB9LFxuICAgIG91dERpcjogJ2Rpc3QnLFxuICAgIGVtcHR5T3V0RGlyOiB0cnVlLFxuICAgIHNvdXJjZW1hcDogdHJ1ZSxcbiAgICBtaW5pZnk6IGZhbHNlLFxuICAgIHRhcmdldDogJ2VzMjAyMCcsXG4gIH0sXG4gIGVzYnVpbGQ6IHtcbiAgICBsb2FkZXI6ICd0cycsXG4gIH0sXG4gIHJlc29sdmU6IHtcbiAgICBleHRlbnNpb25zOiBbJy50cycsICcuanMnXSxcbiAgfSxcbn0pO1xuIl0sCiAgIm1hcHBpbmdzIjogIjtBQUE0VCxTQUFTLG9CQUFvQjtBQUN6VixPQUFPLFNBQVM7QUFFaEIsSUFBTyxzQkFBUSxhQUFhO0FBQUEsRUFDMUIsU0FBUyxDQUFDLElBQUksRUFBRSxhQUFhLEtBQUssQ0FBQyxDQUFDO0FBQUEsRUFDcEMsT0FBTztBQUFBLElBQ0wsS0FBSztBQUFBLE1BQ0gsT0FBTztBQUFBLE1BQ1AsTUFBTTtBQUFBLE1BQ04sVUFBVSxDQUFDLFdBQVcsU0FBUyxXQUFXLE9BQU8sUUFBUSxLQUFLO0FBQUEsTUFDOUQsU0FBUyxDQUFDLE1BQU0sS0FBSztBQUFBLElBQ3ZCO0FBQUEsSUFDQSxlQUFlO0FBQUEsTUFDYixVQUFVO0FBQUEsUUFDUjtBQUFBLFFBQ0E7QUFBQSxRQUNBO0FBQUEsUUFDQTtBQUFBLFFBQ0E7QUFBQSxRQUNBO0FBQUEsUUFDQTtBQUFBLE1BQ0Y7QUFBQSxNQUNBLFFBQVE7QUFBQSxRQUNOLGlCQUFpQjtBQUFBLFFBQ2pCLFNBQVM7QUFBQSxVQUNQLFdBQVc7QUFBQSxVQUNYLG1CQUFtQjtBQUFBLFFBQ3JCO0FBQUEsTUFDRjtBQUFBLElBQ0Y7QUFBQSxJQUNBLFFBQVE7QUFBQSxJQUNSLGFBQWE7QUFBQSxJQUNiLFdBQVc7QUFBQSxJQUNYLFFBQVE7QUFBQSxJQUNSLFFBQVE7QUFBQSxFQUNWO0FBQUEsRUFDQSxTQUFTO0FBQUEsSUFDUCxRQUFRO0FBQUEsRUFDVjtBQUFBLEVBQ0EsU0FBUztBQUFBLElBQ1AsWUFBWSxDQUFDLE9BQU8sS0FBSztBQUFBLEVBQzNCO0FBQ0YsQ0FBQzsiLAogICJuYW1lcyI6IFtdCn0K
