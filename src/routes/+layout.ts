// Tauri doesn't have a Node.js server to do proper SSR
// so we use adapter-static with a fallback to index.html to put the site in SPA mode
// See: https://svelte.dev/docs/kit/single-page-apps
// See: https://v2.tauri.app/start/frontend/sveltekit/ for more info
export const ssr = false;

// In a plain browser there is no back end. In dev, or in a build made with
// "vite build --mode mock", a sample one takes its place so the screens can
// be seen without Tauri. The first half of the condition is fixed at build
// time, so a normal build leaves the sample data out altogether.
export async function load() {
  if ((import.meta.env.DEV || import.meta.env.MODE === "mock") && !("__TAURI_INTERNALS__" in window)) {
    const { installMock } = await import("$lib/mock");
    installMock();
  }
}
