// Rasterise the social card: src/assets/og.svg in, public/og.png out. Run with
// `npm run og` after editing the SVG. The PNG is checked in, so a build never
// depends on this having been run.
import { fileURLToPath } from "node:url";
import sharp from "sharp";

const svg = fileURLToPath(new URL("../src/assets/og.svg", import.meta.url));
const png = fileURLToPath(new URL("../public/og.png", import.meta.url));

await sharp(svg).resize(1200, 630).png({ compressionLevel: 9 }).toFile(png);
console.log("wrote public/og.png");
