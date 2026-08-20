// @ts-check
import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";

// docs.lumitlab.com - its own Cloudflare Pages project so the subdomain is a real
// deployment target rather than a redirect. Shares the marketing site's palette.
export default defineConfig({
  site: "https://docs.lumitlab.com",
  integrations: [
    starlight({
      title: "Lumit docs",
      description: "Documentation for Lumit, the native motion-graphics and compositing editor.",
      logo: { src: "./src/assets/lumit-mark.svg", alt: "Lumit" },
      favicon: "/lumit-mark.svg",
      customCss: ["./src/styles/theme.css"],
      // Git-based per-page dates, shown under the title by the PageTitle
      // override. CI must clone full history or every page shows deploy day.
      lastUpdated: true,
      components: {
        PageTitle: "./src/components/PageTitle.astro",
        LastUpdated: "./src/components/LastUpdated.astro",
      },
      social: [
        { icon: "github", label: "GitHub", href: "https://github.com/luminalmvm/Lumit" },
      ],
      editLink: {
        baseUrl: "https://github.com/luminalmvm/Lumit/edit/main/web-docs/",
      },
      // Using the application comes first and stays first. The engine section is
      // background reading - nobody needs it to edit, so it sits below.
      sidebar: [
        { label: "Start here", autogenerate: { directory: "start" } },
        { label: "Using Lumit", autogenerate: { directory: "use" } },
        { label: "The panels", autogenerate: { directory: "panels" } },
        // One page per effect, generated from the engine's own catalogue by
        // scripts/gen-effect-pages.mjs. Long, so it starts collapsed, and each
        // category is its own collapsed subgroup.
        {
          label: "Effects",
          collapsed: true,
          autogenerate: { directory: "effects", collapsed: true },
        },
        { label: "How Lumit works", autogenerate: { directory: "engine" } },
        { label: "Reference", autogenerate: { directory: "reference" } },
      ],
    }),
  ],
});
