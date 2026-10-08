// What the hero's timeline tells the field behind it, once a frame
// (HeroTimeline.astro writes, HeroField.astro reads). The values standing here
// are what the field draws when there is no timeline to say otherwise.
export const hero = {
  /** How bright the light the lockup throws is. */
  glow: 0.6,
  /** The streak across the lockup as it lands, 0 to 1. */
  flare: 0,
  /** Where the wordmark is between the icon (0) and the word (1). */
  pose: 1,
  /** Whether the field itself is drawn, 1 or 0. */
  field: 1,
  /** Set by the field: draw one frame now. For when nothing is animating. */
  redraw: null as null | (() => void),
  /** The hero holds still: something has come forward over the page. */
  paused: false,
};
