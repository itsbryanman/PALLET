import { defineConfig } from "vite";

// `vite build --mode demo` produces the browser demo (virtual files only),
// served from GitHub Pages under /PALLET/. The default build is what Tauri bundles.
export default defineConfig(({ mode }) => ({
  base: mode === "demo" ? "/PALLET/" : "./",
  clearScreen: false,
  define: {
    __PALLET_DEMO__: JSON.stringify(mode === "demo"),
  },
  server: { port: 1420, strictPort: true },
  build: {
    outDir: mode === "demo" ? "dist-demo" : "dist",
    target: "es2022",
    chunkSizeWarningLimit: 1200,
  },
  test: {
    include: ["tests/**/*.test.ts"],
  },
}));
