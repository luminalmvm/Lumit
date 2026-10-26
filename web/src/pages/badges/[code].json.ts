import type { APIContext } from "astro";
import { LANGUAGES, keys, totals } from "../../scripts/translations";

// How far each translation has got, in the shape shields.io reads, so the README
// can show a badge per language that moves with the next deploy.
export const getStaticPaths = () => LANGUAGES.map((l) => ({ params: { code: l.code } }));

export function GET({ params }: APIContext) {
  const l = LANGUAGES.find((l) => l.code === params.code)!;
  const pct = Math.floor((totals[l.code] / keys.length) * 100);
  // shields.io draws Arabic letters unjoined, so that one goes by its English name.
  const label = l.code === "ar" ? l.name : l.endonym;
  return Response.json({ schemaVersion: 1, label, message: `${pct}%`, color: "35785e" }); // the site's spruce
}
