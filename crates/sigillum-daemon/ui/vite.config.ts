import { defineConfig } from "vite";

export default defineConfig({
  build: {
    outDir: "src",
    emptyOutDir: false,
    rollupOptions: {
      input: "src/app.ts",
      output: {
        entryFileNames: "app.js",
      },
    },
  },
});
