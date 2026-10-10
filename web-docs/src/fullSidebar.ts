import { defineRouteMiddleware, type StarlightRouteData } from "@astrojs/starlight/route-data";

declare global {
  namespace App {
    interface Locals {
      /** Every topic's sidebar, one group a topic, in the order of astro.config.mjs. */
      fullSidebar: StarlightRouteData["sidebar"];
    }
  }
}

// The topics plugin cuts the sidebar down to the topic a page is in. The home
// page lists every topic, so the whole sidebar is kept here before that happens.
export const onRequest = defineRouteMiddleware((context) => {
  context.locals.fullSidebar = context.locals.starlightRoute.sidebar;
});
