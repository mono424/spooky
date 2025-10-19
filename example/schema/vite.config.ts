import { defineConfig, Plugin } from "vite";
import { exec, execSync, spawn, ChildProcess } from "child_process";
import { watch } from "fs";
import path from "path";

let isRestarting = false;
let logsProcess: ChildProcess | null = null;

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

  const startDockerLogs = () => {
    // Stop existing logs process if any
    if (logsProcess) {
      logsProcess.kill();
    }

    // Start docker-compose logs in follow mode
    logsProcess = spawn("docker-compose", ["logs", "-f"], {
      stdio: ["ignore", "inherit", "inherit"],
    });

    logsProcess.on("error", (error) => {
      console.error("❌ Failed to start docker-compose logs:", error);
    });
  };

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

          // Start streaming logs
          console.log("📋 Streaming docker-compose logs...\n");
          startDockerLogs();

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

      // Stop logs process
      if (logsProcess) {
        logsProcess.kill();
        logsProcess = null;
      }

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
        schemaWatcher = null;
      }
    },
    closeBundle() {
      // Keep docker-compose running in the background
      // Logs will continue to stream to the terminal
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
