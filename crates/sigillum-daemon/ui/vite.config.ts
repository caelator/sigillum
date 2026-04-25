import { defineConfig } from "vite";

export default defineConfig({
  build: {
    outDir: "dist",
    emptyOutDir: true,
    rollupOptions: {
      input: "src/app.js",
      output: {
        entryFileNames: "app.js",
      },
    },
  },
});
