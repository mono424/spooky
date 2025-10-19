import { defineConfig, Plugin } from "vite";
import { exec, execSync } from "child_process";
import { watch } from "fs";
import path from "path";

let isRestarting = false;

// Plugin to run build:ts after each build
function buildTsPlugin(): Plugin {
  return {
    name: "build-ts",
    closeBundle() {
      try {
        console.log("\n🔄 Running build:ts...");
        execSync("pnpm run build:ts", { stdio: "inherit" });
        console.log("✅ build:ts completed\n");
      } catch (error) {
        console.error("❌ build:ts failed:", error);
      }
    },
  };
}

// Plugin to manage docker-compose
function dockerComposePlugin(): Plugin {
  let schemaWatcher: ReturnType<typeof watch> | null = null;

  const startDockerCompose = () => {
    return new Promise<void>((resolve, reject) => {
      console.log("\n🐳 Starting docker-compose...");
      exec("docker-compose up -d", (error, stdout) => {
        if (error) {
          console.error("❌ Failed to start docker-compose:", error);
          reject(error);
        } else {
          console.log("✅ docker-compose started\n");
          if (stdout) console.log(stdout);
          resolve();
        }
      });
    });
  };

  const restartDockerCompose = async () => {
    if (isRestarting) return;
    isRestarting = true;

    try {
      console.log("\n🔄 Restarting docker-compose...");

      // Stop and remove containers
      execSync("docker-compose down", { stdio: "inherit" });

      // Start again
      await startDockerCompose();

      console.log("✅ docker-compose restarted\n");
    } catch (error) {
      console.error("❌ Failed to restart docker-compose:", error);
    } finally {
      isRestarting = false;
    }
  };

  return {
    name: "docker-compose",
    async buildStart() {
      // Only run in watch mode (dev)
      if (this.meta.watchMode) {
        await startDockerCompose();

        // Watch schema.surql for changes
        const schemaPath = path.resolve(process.cwd(), "schema.surql");
        console.log(`👀 Watching ${schemaPath} for changes...\n`);

        schemaWatcher = watch(schemaPath, (eventType) => {
          if (eventType === "change") {
            console.log(
              "\n📝 schema.surql changed, restarting docker-compose..."
            );
            restartDockerCompose();
          }
        });
      }
    },
    buildEnd() {
      // Clean up on build end
      if (schemaWatcher) {
        schemaWatcher.close();
      }
    },
    closeBundle() {
      // Don't stop docker-compose in watch mode, only on final exit
      if (!this.meta.watchMode) {
        // In production build, don't manage docker at all
      }
    },
  };
}

export default defineConfig({
  plugins: [dockerComposePlugin(), buildTsPlugin()],
  build: {
    lib: {
      entry: "index.ts",
      name: "Database",
      fileName: (format) => `index.${format === "es" ? "mjs" : "js"}`,
      formats: ["es", "cjs"],
    },
    rollupOptions: {
      external: [],
      output: {
        preserveModules: false,
      },
    },
    outDir: "dist",
    emptyOutDir: true,
  },
  esbuild: {
    loader: "ts",
  },
  resolve: {
    extensions: [".ts", ".js"],
  },
});
