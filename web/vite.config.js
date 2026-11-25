import { defineConfig } from "vite";
import tailwind from "@tailwindcss/vite";

export default defineConfig({
  plugins: [tailwind()],
  build: {
    rolldownOptions: {
      // This workbench runs entirely in the browser. React Server Component
      // directives shipped by UI libraries have no client/server boundary here.
      onLog(level, log, forward) {
        if (log.code === "MODULE_LEVEL_DIRECTIVE" && log.message.includes("use client")) return;
        forward(level, log);
      },
      output: {
        codeSplitting: { groups: [{ name: "markdown", test: /node_modules\/(?:react-markdown|unified|remark-|mdast-|micromark)/ }] },
      },
    },
  },
});
