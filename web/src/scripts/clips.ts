// A clip plays while it is on screen and stops when it is not, so a page of
// them costs what the ones in view cost. Nothing is fetched until a clip
// first comes into view. A visitor who has asked for less motion gets the
// pictures. A workspace behind the front one in Meet the interface keeps
// still, and its clip starts when it is brought forward.
const clips = document.querySelectorAll<HTMLVideoElement>("[data-clip]");
if (clips.length && !matchMedia("(prefers-reduced-motion: reduce)").matches) {
  const seen = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        const clip = entry.target as HTMLVideoElement;
        const behind = clip.closest("[data-layer]:not([aria-current])");
        if (entry.isIntersecting && !behind) clip.play().catch(() => {});
        else clip.pause();
      }
    },
    { threshold: 0.35 },
  );
  for (const clip of clips) {
    clip.addEventListener("playing", () => clip.classList.add("playing"), { once: true });
    seen.observe(clip);
  }
}
