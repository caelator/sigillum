import { defineConfig } from "vite";

export default defineConfig({
  build: {
    outDir: "src",
    emptyOutDir: false,
    cssCodeSplit: true,
    rollupOptions: {
      input: "src/app.ts",
      output: {
        entryFileNames: "app.js",
        assetFileNames: (assetInfo) =>
          assetInfo.name?.endsWith(".css") ? "styles.css" : "[name][extname]",
      },
    },
  },
});
