import { readFileSync } from "node:fs";

// What each language has and is missing, worked out at build time for the
// translation page and the README's badges.
//
// The files are read from the repository rather than copied into web/, so a string
// added to app_en.arb is offered the next time the site deploys - which is on push.
// Cloudflare Pages clones the whole repository and builds from web/, so the relative
// path up to flutter_ui/ holds there as well as locally.
const l10n = (name: string): Record<string, any> =>
  JSON.parse(
    readFileSync(new URL(`../../../flutter_ui/lib/l10n/${name}`, import.meta.url), "utf8"),
  );

const en = l10n("app_en.arb");
// scripts/translations.ps1's sidecar: the English each existing translation was made
// from. It is the only way to know a translation has gone stale.
const state = l10n("translation-state.json");

export const LANGUAGES = [
  { code: "ar", name: "Arabic", endonym: "العربية" },
  { code: "de", name: "German", endonym: "Deutsch" },
  { code: "es", name: "Spanish", endonym: "Español" },
  { code: "it", name: "Italian", endonym: "Italiano" },
  { code: "kk", name: "Kazakh", endonym: "Қазақша" },
  { code: "pl", name: "Polish", endonym: "Polski" },
  { code: "pt", name: "Portuguese, Brazilian", endonym: "Português (Brasil)" },
  { code: "uk", name: "Ukrainian", endonym: "Українська" },
  { code: "zh", name: "Chinese, Simplified", endonym: "简体中文" },
  { code: "zh_Hant", name: "Chinese, Traditional", endonym: "繁體中文" },
];

export const keys = Object.keys(en).filter((k) => !k.startsWith("@"));

// Three parallel arrays rather than 2,278 little objects: the same data, a third of the
// bytes, and the page ships all of it inline so there is nothing to fetch.
export const payload = {
  k: keys,
  s: keys.map((k) => en[k] as string),
  d: keys.map((k) => (en["@" + k]?.description as string) ?? ""),
  loc: {} as Record<
    string,
    { need: number[]; old: Record<number, string>; was: Record<number, string>; has: number[]; t: string[]; m: number }
  >,
};

for (const { code } of LANGUAGES) {
  const arb = l10n(`app_${code}.arb`);
  const seen = state[code] ?? {};
  const need: number[] = [];
  const old: Record<number, string> = {};
  const was: Record<number, string> = {};
  const has: number[] = [];
  keys.forEach((key, i) => {
    const have = arb[key];
    // A key whose "translation" is the English is not translated: the exports these
    // files came from wrote the source text through wherever nobody had answered.
    // Unless the sidecar says someone answered it deliberately - sRGB is sRGB in
    // German too, and an ingested identical answer is an answer.
    if (have === undefined || (have === en[key] && seen[key] === undefined)) {
      need.push(i);
      return;
    }
    const source = seen[key];
    if (source !== undefined && source !== en[key]) {
      need.push(i);
      old[i] = have; // what it says now, so a stale row is a correction and not a retype
      was[i] = source; // and the English that answer was written for
    } else has.push(i);
  });
  // The rows the machine wrote go first, to be checked first, so `m` is how many of `has` they are.
  const machine = (i: number) => seen["machine:" + keys[i]] === true;
  has.sort((a, b) => Number(machine(b)) - Number(machine(a)) || a - b);
  const m = has.filter(machine).length;
  payload.loc[code] = { need, old, was, has, t: has.map((i) => arb[keys[i]] as string), m };
}

export const totals = Object.fromEntries(
  LANGUAGES.map((l) => [l.code, keys.length - payload.loc[l.code].need.length]),
);
