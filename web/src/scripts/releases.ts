import { getCollection } from "astro:content";

// Newest first. Two releases can share a day - 0.3.0 and 0.3.1 did - and a
// sort on the date alone then leaves them in whatever order the loader read
// the files, which put 0.3.1 under 0.3.0. So the date decides, and the version
// breaks the tie, compared as numbers so 0.10.0 sorts above 0.9.0.
const versionParts = (v: string) => v.split(".").map((n) => parseInt(n, 10) || 0);
const byVersionDesc = (a: string, b: string) => {
  const [pa, pb] = [versionParts(a), versionParts(b)];
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const d = (pb[i] ?? 0) - (pa[i] ?? 0);
    if (d !== 0) return d;
  }
  return 0;
};

export const releasesNewestFirst = async () =>
  (await getCollection("releases")).sort(
    (a, b) =>
      +b.data.date - +a.data.date || byVersionDesc(a.data.versionNumber, b.data.versionNumber),
  );
