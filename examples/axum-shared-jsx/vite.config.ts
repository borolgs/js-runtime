import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// https://vite.dev/config/
export default defineConfig({
  root: "src-web",
  base: "app",
  build: {
    // emptyOutDir: true,
    outDir: "../static/app",
  },
  plugins: [react()],
});
