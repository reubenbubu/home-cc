import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";
// @ts-expect-error type error without @types/node package
import process from "node:process";
// @ts-expect-error type error without @types/node package
import { execSync } from "node:child_process";
const host = process.env.TAURI_DEV_HOST;

/** The commit being built, for problem reports, with "+ local changes" when the checkout differs from it.
 *  Empty without git (the local CI builds from a copy without it). */
function commit() {
  const git = (args) => execSync(`git ${args}`, { stdio: ["ignore", "pipe", "ignore"] }).toString().trim();
  try {
    const hash = git("rev-parse --short HEAD");
    return git("status --porcelain --untracked-files=no") ? `${hash} + local changes` : hash;
  } catch {
    return "";
  }
}

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [sveltekit()],
  define: { __HOME_CC_COMMIT__: JSON.stringify(commit()) },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || "127.0.0.1",
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
