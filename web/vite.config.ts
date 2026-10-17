import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Built into the app bundle and served from `amgi://app/`, so asset paths are relative.
export default defineConfig({
  plugins: [svelte()],
  base: "./",
  build: {
    target: "safari18",
    outDir: "dist",
    emptyOutDir: true,
  },
});
