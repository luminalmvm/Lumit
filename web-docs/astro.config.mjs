// @ts-check
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import starlightSidebarTopics from "starlight-sidebar-topics";

// The effect categories come from the engine's own catalogue, so the sidebar can
// say "Blur & sharpen" where the folder on disk says "blur-sharpen". Read once,
// here, and skip any category that has no generated pages on disk yet.
const fxReference = JSON.parse(
  readFileSync(new URL("../crates/lumit-core/fx-reference.json", import.meta.url), "utf8"),
);
const effectCategories = fxReference.categories
  .filter((c) =>
    existsSync(fileURLToPath(new URL(`./src/content/docs/effects/${c.slug}`, import.meta.url))),
  )
  .map((c) => ({
    label: c.label,
    collapsed: true,
    autogenerate: { directory: `effects/${c.slug}` },
  }));

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
      // The before-and-after wipe on every effect page. Each figure carries a
      // range input across the picture; this points the CSS clip at its value.
      // Small enough to inline, and the figures degrade to an honest
      // half-and-half split if it never runs.
      head: [
        {
          tag: "script",
          content: `addEventListener("DOMContentLoaded",function(){
document.querySelectorAll(".compare").forEach(function(c){
var r=c.querySelector(".compare__range");if(!r)return;
var set=function(){c.style.setProperty("--split",r.value+"%")};
r.addEventListener("input",set);set()})})`,
        },
      ],
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
      // The sidebar is split into three tabs across the top of the page, each
      // with its own list underneath. There is no `sidebar` option here on
      // purpose - the plugin below owns it, and Starlight refuses both at once.
      plugins: [
        starlightSidebarTopics(
          [
            // Read in order, once: install it, learn the shape of a composition.
            {
              label: "Tutorials",
              link: "/start/install/",
              icon: "open-book",
              items: [{ label: "Start here", autogenerate: { directory: "start" } }],
            },
            // The working manual. Five groups roughly in the order a shot is
            // made, then one page per panel of the application.
            {
              label: "Guides",
              link: "/use/projects/",
              icon: "pencil",
              items: [
                {
                  label: "Using Lumit",
                  items: [
                    {
                      label: "Projects and media",
                      items: ["use/projects", "use/importing", "use/compositions", "use/export"],
                    },
                    {
                      label: "Layers",
                      items: [
                        "use/layers",
                        "use/transform",
                        "use/blend-modes",
                        "use/masks",
                        "use/mattes",
                        "use/sequence-layers",
                        "use/text",
                        "use/shapes",
                        "use/paint",
                        "use/camera",
                      ],
                    },
                    {
                      label: "Animation",
                      items: [
                        "use/keyframes",
                        "use/graph-editor",
                        "use/retime",
                        "use/expressions",
                        "use/markers",
                        "use/audio",
                      ],
                    },
                    {
                      label: "Effects",
                      items: ["use/effects", "use/fx-console", "use/presets"],
                    },
                    {
                      label: "The application",
                      items: ["use/preview", "use/settings", "use/workspaces"],
                    },
                  ],
                },
                { label: "The panels", autogenerate: { directory: "panels" } },
              ],
            },
            // Look-up material: the effect catalogue, generated one page per
            // effect by scripts/gen-effect-pages.mjs, and the background reading.
            {
              label: "Reference",
              link: "/effects/",
              icon: "information",
              items: [
                {
                  label: "Effects",
                  collapsed: true,
                  items: [{ slug: "effects" }, ...effectCategories],
                },
                { label: "How Lumit works", autogenerate: { directory: "engine" } },
                { label: "Reference", autogenerate: { directory: "reference" } },
              ],
            },
          ],
          // The site root is a landing page and belongs to no topic.
          { exclude: ["/"] },
        ),
      ],
    }),
  ],
});
