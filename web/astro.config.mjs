// @ts-check
import { defineConfig } from "astro/config";
import tailwindcss from "@tailwindcss/vite";
import sitemap from "@astrojs/sitemap";

// Static output: every page is prerendered to HTML at build time and served from
// Cloudflare's CDN. No server, no runtime - the download page's only dynamic bit
// is a client-side fetch of the GitHub releases API.
export default defineConfig({
  site: "https://lumitlab.com",
  output: "static",
  // Writes sitemap-index.xml, which public/robots.txt points crawlers at.
  integrations: [sitemap()],
  vite: { plugins: [tailwindcss()] },
});
