# Colour themes

Thirteen theme pairs for the scheme picker, each a light and a dark variant of one idea. Every value below was computed, not judged by eye: the script that built the tables also rebuilt the mockup page from the same numbers. The first nine pairs are general themes. The last four are drawn from one painted world of wood, brass, parchment and gemstones, and sit in their own section so they can be dropped as a group.

## The rules every scheme meets

- Contrast, WCAG 2.1 relative luminance, measured against `surface1`: `textPrimary` and `textSecondary` at least 7:1, `textMuted` at least 4.5:1, `textDisabled` at least 3:1, and every non-text boundary (`accent`, `success`, `warning`, `error`, `hairlineStrong`) at least 3:1. The figure sits beside every value.
- `viewerSurround` is strictly neutral, R = G = B: `#1c1c1c` in every dark scheme and `#a8a8a8` in every light one. It is a grading surface and never takes the tint.
- The six layer hues are muted, mid-lightness siblings. Their chroma sits at least 12 CIELAB units under the accent, and the accent stays at least 10 dE76 from every one of them under every deficiency simulated below, so selection always wins.
- The six layer hues step monotonically in lightness, solid darkest and text lightest, in the same order in every scheme: solid, precomp, sequence, footage, camera, text. The minimum step is 8 L\*, and 10 L\* in Chalk. Each table states Y and L\* for every step.
- `error`, `success` and `warning` step in lightness by at least 10 L\*, error darkest, in every scheme, so a red and green confusion still leaves a dark and a light one.
- `accent` is at least 40 degrees of hue or 12 L\* away from `success`.
- `accentHover` is `accent` shifted 0x12 per channel, lighter on dark and darker on light, clamped to the channel.
- `curve[0..3]` and the layer hues never reuse `accent`, `success`, `warning` or `error`.
- Light schemes are re-derived at the light end. `surface1` is the panel, `surface0` and `surface4` are darker washes, and `surface3` is the floating white.
- Slate is monochrome: every surface, text step and hairline is R = G = B, and the check fails if one drifts. Colour appears there only in the accent, the roles, the layer family and the curve ramp.
- Colour deficiency. Every colour was passed through the Machado, Oliveira and Fernandes 2009 matrices at full severity for protanopia, deuteranopia and tritanopia, then compared in CIELAB (dE76). The check passes when the closest pair of layer hues is at least 7 apart, the closest pair of error, success and warning is at least 12 apart, and the accent is at least 10 from success and from every layer hue. The tables show the actual minimum for each deficiency. The tightest figure everywhere is a blue accent against the blue layer bars under tritanopia, where the accent keeps only its lightness and a little red; the floor of 10 is met in every scheme, and the selection outline, the playhead triangle and the filled button carry the rest by shape.

Layer hues are built in CIE LCh: a stated lightness, a stated hue, and a chroma that is reduced until the colour is inside sRGB, so the ladder is exact and the family stays muted. The hexes in the tables are the rounded results.

## Mallow

Soft and pastel, for someone who wants the editor to feel gentle rather than technical. The greys carry a faint mauve, the accent is a lavender, and the role colours are the tints of sugared almonds: coral, mint and peach, kept quiet enough that a full panel never looks sweet.

### Mallow dark

| Token | Value | Y | Against surface1 | Floor |
|---|---|---|---|---|
| `surface0` | `#1c1920` | 0.0105 | 1.10:1 |  |
| `surface1` | `#262129` | 0.0166 | ground |  |
| `surface2` | `#2f2933` | 0.0243 | 1.12:1 |  |
| `surface3` | `#383140` | 0.0341 | 1.26:1 |  |
| `surface4` | `#443b4d` | 0.0489 | 1.49:1 |  |
| `viewerSurround` | `#1c1c1c` | 0.0116 | neutral, R = G = B |  |
| `textPrimary` | `#f2ecf2` | 0.8528 | 13.56:1 | 7.0:1 |
| `textSecondary` | `#d6ccd8` | 0.6244 | 10.13:1 | 7.0:1 |
| `textMuted` | `#a99bb1` | 0.3505 | 6.01:1 | 4.5:1 |
| `textDisabled` | `#7c6d84` | 0.1689 | 3.29:1 | 3.0:1 |
| `hairline` | `#362f3c` | 0.0314 | 1.22:1 |  |
| `hairlineStrong` | `#82748b` | 0.1910 | 3.62:1 | 3.0:1 |
| `accent` | `#bda3f2` | 0.4342 | 7.27:1 | 3.0:1 |
| `accentHover` | `#cfb5ff` | 0.5353 | 8.79:1 |  |
| `animated` | `#d8b071` | 0.4684 | 7.78:1 |  |
| `success` | `#74c09a` | 0.4375 | 7.32:1 | 3.0:1 |
| `warning` | `#f8d198` | 0.6782 | 10.93:1 | 3.0:1 |
| `error` | `#c86792` | 0.2406 | 4.36:1 | 3.0:1 |
| `cacheDisk` | `#6eb3de` | 0.4083 | 6.88:1 |  |

Curve ramp: `#72d5e1`, `#bdcd95`, `#eca8c6`, `#dfcb94`.

Layer ladder, darkest to lightest. Minimum step 8 L\*.

| Layer | Value | Y | L\* | Step | Chroma | Against surface1 |
|---|---|---|---|---|---|---|
| `layer.solid` | `#4d4a50` | 0.0705 | 31.9 |  | 4.0 | 1.81:1 |
| `layer.precomp` | `#6f5768` | 0.1120 | 39.9 | +8.0 | 14.3 | 2.43:1 |
| `layer.sequence` | `#6f7088` | 0.1675 | 47.9 | +8.0 | 14.2 | 3.27:1 |
| `layer.footage` | `#6a8b9a` | 0.2386 | 55.9 | +8.0 | 14.1 | 4.33:1 |
| `layer.camera` | `#ae9785` | 0.3283 | 64.0 | +8.1 | 13.8 | 5.68:1 |
| `layer.text` | `#bbb097` | 0.4385 | 72.1 | +8.1 | 14.2 | 7.33:1 |

Accent chroma 44.1 against a highest layer chroma of 14.3.

Role ladder: error L\* 56.1, success L\* 72.1, warning L\* 85.9. Accent to success: 145 degrees of hue, 0.2 L\*.

| Simulated | Closest layer pair | Closest role pair | Accent to success | Accent to nearest layer |
|---|---|---|---|---|
| Protanopia | 8.5 | 19.0 | 54.7 | 31.8 |
| Deuteranopia | 7.4 | 16.2 | 45.0 | 28.2 |
| Tritanopia | 10.0 | 39.4 | 35.1 | 12.4 |

### Mallow light

| Token | Value | Y | Against surface1 | Floor |
|---|---|---|---|---|
| `surface0` | `#ebe4ec` | 0.7921 | 1.18:1 |  |
| `surface1` | `#fbf8fb` | 0.9461 | ground |  |
| `surface2` | `#f3eef4` | 0.8674 | 1.09:1 |  |
| `surface3` | `#fdfbfd` | 0.9697 | 1.02:1 |  |
| `surface4` | `#e6dde8` | 0.7436 | 1.26:1 |  |
| `viewerSurround` | `#a8a8a8` | 0.3916 | neutral, R = G = B |  |
| `textPrimary` | `#2b2430` | 0.0199 | 14.25:1 | 7.0:1 |
| `textSecondary` | `#4d4354` | 0.0623 | 8.87:1 | 7.0:1 |
| `textMuted` | `#766a7d` | 0.1564 | 4.83:1 | 4.5:1 |
| `textDisabled` | `#93869a` | 0.2559 | 3.26:1 | 3.0:1 |
| `hairline` | `#ddd4de` | 0.6773 | 1.37:1 |  |
| `hairlineStrong` | `#8f8397` | 0.2431 | 3.40:1 | 3.0:1 |
| `accent` | `#7e59ab` | 0.1452 | 5.10:1 | 3.0:1 |
| `accentHover` | `#6c4799` | 0.0999 | 6.64:1 |  |
| `animated` | `#9a6f27` | 0.1839 | 4.26:1 |  |
| `success` | `#2e805c` | 0.1679 | 4.57:1 | 3.0:1 |
| `warning` | `#b48237` | 0.2594 | 3.22:1 | 3.0:1 |
| `error` | `#9c3066` | 0.1014 | 6.58:1 | 3.0:1 |
| `cacheDisk` | `#2a79a1` | 0.1674 | 4.58:1 |  |

Curve ramp: `#02848f`, `#6a7e40`, `#a55a7d`, `#8d7b3d`.

Layer ladder, darkest to lightest. Minimum step 8 L\*.

| Layer | Value | Y | L\* | Step | Chroma | Against surface1 |
|---|---|---|---|---|---|---|
| `layer.solid` | `#444147` | 0.0546 | 28.0 |  | 4.1 | 9.52:1 |
| `layer.precomp` | `#674d60` | 0.0904 | 36.1 | +8.0 | 16.0 | 7.10:1 |
| `layer.sequence` | `#646681` | 0.1380 | 43.9 | +7.9 | 16.2 | 5.30:1 |
| `layer.footage` | `#5b8292` | 0.2026 | 52.1 | +8.2 | 15.9 | 3.94:1 |
| `layer.camera` | `#a68c77` | 0.2820 | 60.1 | +7.9 | 16.2 | 3.00:1 |
| `layer.text` | `#b1a589` | 0.3806 | 68.1 | +8.0 | 16.1 | 2.31:1 |

Accent chroma 50.2 against a highest layer chroma of 16.2.

Role ladder: error L\* 38.1, success L\* 48.0, warning L\* 58.0. Accent to success: 150 degrees of hue, 3.0 L\*.

| Simulated | Closest layer pair | Closest role pair | Accent to success | Accent to nearest layer |
|---|---|---|---|---|
| Protanopia | 8.9 | 30.4 | 58.7 | 26.0 |
| Deuteranopia | 7.5 | 12.7 | 47.7 | 22.5 |
| Tritanopia | 10.6 | 29.7 | 41.6 | 13.8 |

## Glacier

Cool and exact, for a technical room. Blue-black surfaces, a cyan accent on dark and a clear blue on light, since a cyan dark enough to hold on white goes grey, and role colours that read like status lamps: a rose for errors, a clear green for success and a plain yellow for warnings. The layer hues sit on the blue side of the wheel so the timeline stays cold and orderly.

### Glacier dark

| Token | Value | Y | Against surface1 | Floor |
|---|---|---|---|---|
| `surface0` | `#0c1014` | 0.0050 | 1.07:1 |  |
| `surface1` | `#12181e` | 0.0088 | ground |  |
| `surface2` | `#182027` | 0.0137 | 1.08:1 |  |
| `surface3` | `#1e2830` | 0.0201 | 1.19:1 |  |
| `surface4` | `#28343e` | 0.0325 | 1.40:1 |  |
| `viewerSurround` | `#1c1c1c` | 0.0116 | neutral, R = G = B |  |
| `textPrimary` | `#e9f0f5` | 0.8624 | 15.53:1 | 7.0:1 |
| `textSecondary` | `#bccad4` | 0.5769 | 10.67:1 | 7.0:1 |
| `textMuted` | `#8397a5` | 0.2968 | 5.90:1 | 4.5:1 |
| `textDisabled` | `#5d6e7a` | 0.1488 | 3.38:1 | 3.0:1 |
| `hairline` | `#222c35` | 0.0240 | 1.26:1 |  |
| `hairlineStrong` | `#5d7280` | 0.1592 | 3.56:1 | 3.0:1 |
| `accent` | `#54cae4` | 0.4973 | 9.31:1 | 3.0:1 |
| `accentHover` | `#66dcf6` | 0.6067 | 11.18:1 |  |
| `animated` | `#dfb56a` | 0.4978 | 9.32:1 |  |
| `success` | `#4fb985` | 0.3805 | 7.33:1 | 3.0:1 |
| `warning` | `#f7d46a` | 0.6790 | 12.41:1 | 3.0:1 |
| `error` | `#d1568b` | 0.2207 | 4.61:1 | 3.0:1 |
| `cacheDisk` | `#569cd4` | 0.3051 | 6.04:1 |  |

Curve ramp: `#62d8db`, `#afd195`, `#eca7d2`, `#dbcc94`.

Layer ladder, darkest to lightest. Minimum step 8 L\*.

| Layer | Value | Y | L\* | Step | Chroma | Against surface1 |
|---|---|---|---|---|---|---|
| `layer.solid` | `#454c51` | 0.0703 | 31.9 |  | 4.2 | 2.05:1 |
| `layer.precomp` | `#605c6f` | 0.1129 | 40.1 | +8.2 | 11.7 | 2.77:1 |
| `layer.sequence` | `#657386` | 0.1675 | 47.9 | +7.9 | 12.2 | 3.70:1 |
| `layer.footage` | `#6c8c92` | 0.2402 | 56.1 | +8.2 | 11.9 | 4.94:1 |
| `layer.camera` | `#ae968a` | 0.3265 | 63.9 | +7.8 | 11.9 | 6.41:1 |
| `layer.text` | `#b8b09b` | 0.4361 | 72.0 | +8.1 | 11.8 | 8.27:1 |

Accent chroma 34.0 against a highest layer chroma of 12.2.

Role ladder: error L\* 54.1, success L\* 68.1, warning L\* 86.0. Accent to success: 64 degrees of hue, 7.8 L\*.

| Simulated | Closest layer pair | Closest role pair | Accent to success | Accent to nearest layer |
|---|---|---|---|---|
| Protanopia | 8.6 | 37.3 | 42.6 | 25.5 |
| Deuteranopia | 7.9 | 16.0 | 41.1 | 25.8 |
| Tritanopia | 9.5 | 50.0 | 12.3 | 34.2 |

### Glacier light

| Token | Value | Y | Against surface1 | Floor |
|---|---|---|---|---|
| `surface0` | `#e3e8ec` | 0.8010 | 1.17:1 |  |
| `surface1` | `#f7f9fb` | 0.9449 | ground |  |
| `surface2` | `#eef2f5` | 0.8827 | 1.07:1 |  |
| `surface3` | `#fbfcfd` | 0.9722 | 1.03:1 |  |
| `surface4` | `#d9e0e6` | 0.7378 | 1.26:1 |  |
| `viewerSurround` | `#a8a8a8` | 0.3916 | neutral, R = G = B |  |
| `textPrimary` | `#16202a` | 0.0137 | 15.62:1 | 7.0:1 |
| `textSecondary` | `#3a4954` | 0.0630 | 8.80:1 | 7.0:1 |
| `textMuted` | `#607280` | 0.1608 | 4.72:1 | 4.5:1 |
| `textDisabled` | `#7d8c98` | 0.2538 | 3.27:1 | 3.0:1 |
| `hairline` | `#d2dae0` | 0.6923 | 1.34:1 |  |
| `hairlineStrong` | `#7c8a96` | 0.2466 | 3.35:1 | 3.0:1 |
| `accent` | `#0075c9` | 0.1694 | 4.53:1 | 3.0:1 |
| `accentHover` | `#0063b7` | 0.1234 | 5.74:1 |  |
| `animated` | `#a17423` | 0.2019 | 3.95:1 |  |
| `success` | `#008353` | 0.1686 | 4.55:1 | 3.0:1 |
| `warning` | `#ab8720` | 0.2609 | 3.20:1 | 3.0:1 |
| `error` | `#a71f63` | 0.1010 | 6.59:1 | 3.0:1 |
| `cacheDisk` | `#1e73a8` | 0.1536 | 4.89:1 |  |

Curve ramp: `#007f82`, `#5b8141`, `#a4598a`, `#897c3d`.

Layer ladder, darkest to lightest. Minimum step 8 L\*.

| Layer | Value | Y | L\* | Step | Chroma | Against surface1 |
|---|---|---|---|---|---|---|
| `layer.solid` | `#3c4347` | 0.0543 | 27.9 |  | 3.9 | 9.54:1 |
| `layer.precomp` | `#575268` | 0.0906 | 36.1 | +8.2 | 13.9 | 7.08:1 |
| `layer.sequence` | `#586a7f` | 0.1392 | 44.1 | +8.0 | 13.9 | 5.26:1 |
| `layer.footage` | `#5c8289` | 0.2005 | 51.9 | +7.8 | 14.0 | 3.97:1 |
| `layer.camera` | `#a68b7d` | 0.2805 | 59.9 | +8.0 | 13.8 | 3.01:1 |
| `layer.text` | `#aea58d` | 0.3783 | 67.9 | +8.0 | 13.6 | 2.32:1 |

Accent chroma 51.3 against a highest layer chroma of 14.0.

Role ladder: error L\* 38.0, success L\* 48.1, warning L\* 58.1. Accent to success: 117 degrees of hue, 0.1 L\*.

| Simulated | Closest layer pair | Closest role pair | Accent to success | Accent to nearest layer |
|---|---|---|---|---|
| Protanopia | 8.9 | 36.7 | 71.4 | 35.6 |
| Deuteranopia | 7.7 | 13.7 | 69.8 | 40.8 |
| Tritanopia | 9.6 | 44.3 | 14.6 | 12.4 |

## Hearth

Warm, earthy and domestic, for evenings at a home desk. Brown-charcoal surfaces like a dark kitchen table, a terracotta accent, and roles drawn from the cupboard: brick, sage and honey. The layer hues lean warm too, so the timeline reads as one material rather than as a chart.

### Hearth dark

| Token | Value | Y | Against surface1 | Floor |
|---|---|---|---|---|
| `surface0` | `#1a1512` | 0.0080 | 1.09:1 |  |
| `surface1` | `#241d18` | 0.0132 | ground |  |
| `surface2` | `#2d251f` | 0.0198 | 1.10:1 |  |
| `surface3` | `#372e27` | 0.0291 | 1.25:1 |  |
| `surface4` | `#443930` | 0.0437 | 1.48:1 |  |
| `viewerSurround` | `#1c1c1c` | 0.0116 | neutral, R = G = B |  |
| `textPrimary` | `#f3ebe2` | 0.8396 | 14.08:1 | 7.0:1 |
| `textSecondary` | `#d9ccbe` | 0.6166 | 10.55:1 | 7.0:1 |
| `textMuted` | `#a99889` | 0.3270 | 5.97:1 | 4.5:1 |
| `textDisabled` | `#7d6e62` | 0.1639 | 3.39:1 | 3.0:1 |
| `hairline` | `#34291f` | 0.0241 | 1.17:1 |  |
| `hairlineStrong` | `#82725f` | 0.1761 | 3.58:1 | 3.0:1 |
| `accent` | `#e48158` | 0.3290 | 6.00:1 | 3.0:1 |
| `accentHover` | `#f6936a` | 0.4150 | 7.36:1 |  |
| `animated` | `#d2b26c` | 0.4663 | 8.17:1 |  |
| `success` | `#76b386` | 0.3781 | 6.77:1 | 3.0:1 |
| `warning` | `#f0c675` | 0.6020 | 10.32:1 | 3.0:1 |
| `error` | `#c25980` | 0.2017 | 3.98:1 | 3.0:1 |
| `cacheDisk` | `#49a6c4` | 0.3267 | 5.96:1 |  |

Curve ramp: `#73d0c9`, `#b7c890`, `#e9a2bc`, `#d5c68f`.

Layer ladder, darkest to lightest. Minimum step 8 L\*.

| Layer | Value | Y | L\* | Step | Chroma | Against surface1 |
|---|---|---|---|---|---|---|
| `layer.solid` | `#504a46` | 0.0705 | 31.9 |  | 3.7 | 1.91:1 |
| `layer.precomp` | `#725764` | 0.1131 | 40.1 | +8.2 | 13.8 | 2.58:1 |
| `layer.sequence` | `#6c7189` | 0.1680 | 48.0 | +7.9 | 14.2 | 3.45:1 |
| `layer.footage` | `#668d92` | 0.2395 | 56.0 | +8.0 | 14.0 | 4.58:1 |
| `layer.camera` | `#b29588` | 0.3274 | 63.9 | +7.9 | 14.0 | 5.97:1 |
| `layer.text` | `#bdaf97` | 0.4371 | 72.0 | +8.1 | 14.2 | 7.71:1 |

Accent chroma 51.8 against a highest layer chroma of 14.2.

Role ladder: error L\* 52.0, success L\* 67.9, warning L\* 81.9. Accent to success: 102 degrees of hue, 3.8 L\*.

| Simulated | Closest layer pair | Closest role pair | Accent to success | Accent to nearest layer |
|---|---|---|---|---|
| Protanopia | 9.7 | 27.0 | 15.6 | 22.5 |
| Deuteranopia | 8.2 | 16.4 | 27.1 | 27.8 |
| Tritanopia | 10.7 | 40.0 | 81.1 | 40.8 |

### Hearth light

| Token | Value | Y | Against surface1 | Floor |
|---|---|---|---|---|
| `surface0` | `#ece5dc` | 0.7904 | 1.17:1 |  |
| `surface1` | `#fbf7f1` | 0.9338 | ground |  |
| `surface2` | `#f3ede5` | 0.8528 | 1.09:1 |  |
| `surface3` | `#fdfaf6` | 0.9591 | 1.03:1 |  |
| `surface4` | `#e4dbcf` | 0.7166 | 1.28:1 |  |
| `viewerSurround` | `#a8a8a8` | 0.3916 | neutral, R = G = B |  |
| `textPrimary` | `#2a221c` | 0.0172 | 14.64:1 | 7.0:1 |
| `textSecondary` | `#4e433a` | 0.0594 | 8.99:1 | 7.0:1 |
| `textMuted` | `#786a5e` | 0.1511 | 4.89:1 | 4.5:1 |
| `textDisabled` | `#978a7e` | 0.2626 | 3.15:1 | 3.0:1 |
| `hairline` | `#dfd5c9` | 0.6749 | 1.36:1 |  |
| `hairlineStrong` | `#8e8071` | 0.2238 | 3.59:1 | 3.0:1 |
| `accent` | `#aa4220` | 0.1255 | 5.61:1 | 3.0:1 |
| `accentHover` | `#98300e` | 0.0882 | 7.12:1 |  |
| `animated` | `#977026` | 0.1831 | 4.22:1 |  |
| `success` | `#3f7f52` | 0.1684 | 4.50:1 | 3.0:1 |
| `warning` | `#b4832e` | 0.2613 | 3.16:1 | 3.0:1 |
| `error` | `#9e2e5e` | 0.1003 | 6.55:1 | 3.0:1 |
| `cacheDisk` | `#007793` | 0.1530 | 4.85:1 |  |

Curve ramp: `#00807a`, `#6a7e40`, `#a85978`, `#897c3d`.

Layer ladder, darkest to lightest. Minimum step 8 L\*.

| Layer | Value | Y | L\* | Step | Chroma | Against surface1 |
|---|---|---|---|---|---|---|
| `layer.solid` | `#47413d` | 0.0546 | 28.0 |  | 3.8 | 9.41:1 |
| `layer.precomp` | `#6a4c5c` | 0.0901 | 36.0 | +8.0 | 16.1 | 7.02:1 |
| `layer.sequence` | `#616781` | 0.1383 | 44.0 | +8.0 | 15.7 | 5.23:1 |
| `layer.footage` | `#568389` | 0.2002 | 51.9 | +7.9 | 16.0 | 3.93:1 |
| `layer.camera` | `#aa8a7c` | 0.2818 | 60.0 | +8.2 | 15.7 | 2.97:1 |
| `layer.text` | `#b4a489` | 0.3806 | 68.1 | +8.0 | 16.2 | 2.28:1 |

Accent chroma 58.0 against a highest layer chroma of 16.2.

Role ladder: error L\* 37.9, success L\* 48.1, warning L\* 58.2. Accent to success: 105 degrees of hue, 6.0 L\*.

| Simulated | Closest layer pair | Closest role pair | Accent to success | Accent to nearest layer |
|---|---|---|---|---|
| Protanopia | 10.2 | 29.8 | 17.6 | 32.2 |
| Deuteranopia | 8.3 | 13.9 | 28.1 | 34.5 |
| Tritanopia | 11.0 | 32.2 | 88.5 | 48.5 |

## Chalk

The accessible pair. Maximum contrast: near-black under near-white, a blue accent that survives all three deficiencies, and roles set on a hard lightness ladder so error, success and warning are told apart with no colour vision at all. The layer hues step by ten points of lightness instead of eight. Pick it for low vision, for colour deficiency, or for a bright room.

### Chalk dark

| Token | Value | Y | Against surface1 | Floor |
|---|---|---|---|---|
| `surface0` | `#060606` | 0.0018 | 1.05:1 |  |
| `surface1` | `#0e0e0e` | 0.0044 | ground |  |
| `surface2` | `#181818` | 0.0091 | 1.09:1 |  |
| `surface3` | `#222222` | 0.0160 | 1.21:1 |  |
| `surface4` | `#2e2e2e` | 0.0273 | 1.42:1 |  |
| `viewerSurround` | `#1c1c1c` | 0.0116 | neutral, R = G = B |  |
| `textPrimary` | `#f8f8f8` | 0.9387 | 18.18:1 | 7.0:1 |
| `textSecondary` | `#e2e2e2` | 0.7605 | 14.90:1 | 7.0:1 |
| `textMuted` | `#b4b4b4` | 0.4564 | 9.31:1 | 4.5:1 |
| `textDisabled` | `#8c8c8c` | 0.2623 | 5.74:1 | 3.0:1 |
| `hairline` | `#343434` | 0.0343 | 1.55:1 |  |
| `hairlineStrong` | `#8c8c8c` | 0.2623 | 5.74:1 | 3.0:1 |
| `accent` | `#7dc1fe` | 0.4966 | 10.05:1 | 3.0:1 |
| `accentHover` | `#8fd3ff` | 0.5965 | 11.89:1 |  |
| `animated` | `#e9c268` | 0.5691 | 11.38:1 |  |
| `success` | `#1cbc81` | 0.3780 | 7.87:1 | 3.0:1 |
| `warning` | `#fbe35f` | 0.7627 | 14.94:1 | 3.0:1 |
| `error` | `#cf4a8f` | 0.2015 | 4.62:1 | 3.0:1 |
| `cacheDisk` | `#5ab7d4` | 0.4079 | 8.42:1 |  |

Curve ramp: `#6fe3e6`, `#badca0`, `#f7b1dd`, `#e6d79f`.

Layer ladder, darkest to lightest. Minimum step 10 L\*.

| Layer | Value | Y | L\* | Step | Chroma | Against surface1 |
|---|---|---|---|---|---|---|
| `layer.solid` | `#3e3e3e` | 0.0482 | 26.2 |  | 0.0 | 1.80:1 |
| `layer.precomp` | `#565265` | 0.0895 | 35.9 | +9.7 | 12.0 | 2.57:1 |
| `layer.sequence` | `#606e81` | 0.1522 | 45.9 | +10.0 | 12.3 | 3.72:1 |
| `layer.footage` | `#6c8c92` | 0.2402 | 56.1 | +10.2 | 11.9 | 5.34:1 |
| `layer.camera` | `#b49c8f` | 0.3546 | 66.1 | +10.0 | 12.1 | 7.44:1 |
| `layer.text` | `#c3bba5` | 0.4986 | 76.0 | +9.9 | 12.2 | 10.09:1 |

Accent chroma 37.0 against a highest layer chroma of 12.3.

Role ladder: error L\* 52.0, success L\* 67.9, warning L\* 90.0. Accent to success: 102 degrees of hue, 8.0 L\*.

| Simulated | Closest layer pair | Closest role pair | Accent to success | Accent to nearest layer |
|---|---|---|---|---|
| Protanopia | 10.7 | 47.1 | 59.9 | 35.1 |
| Deuteranopia | 9.6 | 21.9 | 55.3 | 36.5 |
| Tritanopia | 10.1 | 60.6 | 14.9 | 30.0 |

### Chalk light

| Token | Value | Y | Against surface1 | Floor |
|---|---|---|---|---|
| `surface0` | `#e6e6e6` | 0.7913 | 1.25:1 |  |
| `surface1` | `#ffffff` | 1.0000 | ground |  |
| `surface2` | `#f2f2f2` | 0.8879 | 1.12:1 |  |
| `surface3` | `#ffffff` | 1.0000 | 1.00:1 |  |
| `surface4` | `#dadada` | 0.7011 | 1.40:1 |  |
| `viewerSurround` | `#a8a8a8` | 0.3916 | neutral, R = G = B |  |
| `textPrimary` | `#0a0a0a` | 0.0030 | 19.80:1 | 7.0:1 |
| `textSecondary` | `#262626` | 0.0194 | 15.13:1 | 7.0:1 |
| `textMuted` | `#4a4a4a` | 0.0685 | 8.86:1 | 4.5:1 |
| `textDisabled` | `#6a6a6a` | 0.1441 | 5.41:1 | 3.0:1 |
| `hairline` | `#c8c8c8` | 0.5776 | 1.67:1 |  |
| `hairlineStrong` | `#6e6e6e` | 0.1559 | 5.10:1 | 3.0:1 |
| `accent` | `#004ea1` | 0.0802 | 8.06:1 | 3.0:1 |
| `accentHover` | `#003c8f` | 0.0521 | 10.28:1 |  |
| `animated` | `#936b00` | 0.1672 | 4.83:1 |  |
| `success` | `#008055` | 0.1609 | 4.98:1 | 3.0:1 |
| `warning` | `#a38a00` | 0.2596 | 3.39:1 | 3.0:1 |
| `error` | `#a30c68` | 0.0905 | 7.47:1 | 3.0:1 |
| `cacheDisk` | `#00728b` | 0.1390 | 5.56:1 |  |

Curve ramp: `#017a7c`, `#527d36`, `#a35087`, `#847730`.

Layer ladder, darkest to lightest. Minimum step 10 L\*.

| Layer | Value | Y | L\* | Step | Chroma | Against surface1 |
|---|---|---|---|---|---|---|
| `layer.solid` | `#353535` | 0.0356 | 22.2 |  | 0.0 | 12.27:1 |
| `layer.precomp` | `#4d485e` | 0.0702 | 31.9 | +9.7 | 14.2 | 8.73:1 |
| `layer.sequence` | `#53657a` | 0.1255 | 42.1 | +10.2 | 14.0 | 5.98:1 |
| `layer.footage` | `#5c8289` | 0.2005 | 51.9 | +9.8 | 14.0 | 4.19:1 |
| `layer.camera` | `#ac9082` | 0.3033 | 61.9 | +10.0 | 14.1 | 2.97:1 |
| `layer.text` | `#b9b097` | 0.4360 | 72.0 | +10.0 | 14.0 | 2.16:1 |

Accent chroma 52.3 against a highest layer chroma of 14.2.

Role ladder: error L\* 36.1, success L\* 47.1, warning L\* 58.0. Accent to success: 125 degrees of hue, 13.1 L\*.

| Simulated | Closest layer pair | Closest role pair | Accent to success | Accent to nearest layer |
|---|---|---|---|---|
| Protanopia | 11.1 | 41.1 | 69.8 | 35.5 |
| Deuteranopia | 9.7 | 17.3 | 67.2 | 40.4 |
| Tritanopia | 10.2 | 48.6 | 20.2 | 12.8 |

## Vellum

Quiet and paper-like, for long reading and long sessions. Cream and warm-grey surfaces, an ink-blue accent, and roles no louder than a pencil note: dusty rose, sage and wheat. It is the pair to choose when the footage should be the only colourful thing on the screen.

### Vellum dark

| Token | Value | Y | Against surface1 | Floor |
|---|---|---|---|---|
| `surface0` | `#1e1d1b` | 0.0123 | 1.13:1 |  |
| `surface1` | `#282725` | 0.0204 | ground |  |
| `surface2` | `#32302d` | 0.0298 | 1.13:1 |  |
| `surface3` | `#3b3936` | 0.0412 | 1.30:1 |  |
| `surface4` | `#47443f` | 0.0583 | 1.54:1 |  |
| `viewerSurround` | `#1c1c1c` | 0.0116 | neutral, R = G = B |  |
| `textPrimary` | `#ede8df` | 0.8105 | 12.23:1 | 7.0:1 |
| `textSecondary` | `#cfc8bc` | 0.5820 | 8.98:1 | 7.0:1 |
| `textMuted` | `#a49d90` | 0.3402 | 5.55:1 | 4.5:1 |
| `textDisabled` | `#7b7569` | 0.1795 | 3.26:1 | 3.0:1 |
| `hairline` | `#383632` | 0.0371 | 1.24:1 |  |
| `hairlineStrong` | `#82796d` | 0.1952 | 3.49:1 | 3.0:1 |
| `accent` | `#54aad4` | 0.3539 | 5.74:1 | 3.0:1 |
| `accentHover` | `#66bce6` | 0.4450 | 7.04:1 |  |
| `animated` | `#ccac77` | 0.4367 | 6.92:1 |  |
| `success` | `#7eb191` | 0.3792 | 6.10:1 | 3.0:1 |
| `warning` | `#e8c88d` | 0.6039 | 9.29:1 | 3.0:1 |
| `error` | `#b66a87` | 0.2200 | 3.84:1 | 3.0:1 |
| `cacheDisk` | `#64a5b4` | 0.3291 | 5.39:1 |  |

Curve ramp: `#84c7c8`, `#b5c098`, `#d5a3b8`, `#ccc198`.

Layer ladder, darkest to lightest. Minimum step 8 L\*.

| Layer | Value | Y | L\* | Step | Chroma | Against surface1 |
|---|---|---|---|---|---|---|
| `layer.solid` | `#4f4b45` | 0.0712 | 32.1 |  | 4.2 | 1.72:1 |
| `layer.precomp` | `#6b5964` | 0.1119 | 39.9 | +7.8 | 10.1 | 2.30:1 |
| `layer.sequence` | `#6c7282` | 0.1683 | 48.1 | +8.2 | 9.6 | 3.10:1 |
| `layer.footage` | `#718a92` | 0.2376 | 55.8 | +7.8 | 10.1 | 4.09:1 |
| `layer.camera` | `#aa988c` | 0.3290 | 64.1 | +8.2 | 9.9 | 5.39:1 |
| `layer.text` | `#b8b09e` | 0.4371 | 72.0 | +8.0 | 10.2 | 6.92:1 |

Accent chroma 32.1 against a highest layer chroma of 10.2.

Role ladder: error L\* 54.0, success L\* 68.0, warning L\* 82.0. Accent to success: 90 degrees of hue, 1.9 L\*.

| Simulated | Closest layer pair | Closest role pair | Accent to success | Accent to nearest layer |
|---|---|---|---|---|
| Protanopia | 8.6 | 22.0 | 40.2 | 23.3 |
| Deuteranopia | 7.6 | 14.0 | 41.5 | 25.5 |
| Tritanopia | 9.3 | 33.1 | 18.6 | 27.2 |

### Vellum light

| Token | Value | Y | Against surface1 | Floor |
|---|---|---|---|---|
| `surface0` | `#e9e3d8` | 0.7722 | 1.18:1 |  |
| `surface1` | `#faf6ee` | 0.9241 | ground |  |
| `surface2` | `#f2ecdf` | 0.8420 | 1.09:1 |  |
| `surface3` | `#fcf9f3` | 0.9492 | 1.03:1 |  |
| `surface4` | `#e0d9cc` | 0.6983 | 1.30:1 |  |
| `viewerSurround` | `#a8a8a8` | 0.3916 | neutral, R = G = B |  |
| `textPrimary` | `#2c2a25` | 0.0232 | 13.30:1 | 7.0:1 |
| `textSecondary` | `#4b483f` | 0.0649 | 8.48:1 | 7.0:1 |
| `textMuted` | `#716b60` | 0.1487 | 4.90:1 | 4.5:1 |
| `textDisabled` | `#8f8879` | 0.2483 | 3.27:1 | 3.0:1 |
| `hairline` | `#d9d1c3` | 0.6429 | 1.41:1 |  |
| `hairlineStrong` | `#8a8375` | 0.2292 | 3.49:1 | 3.0:1 |
| `accent` | `#1367a5` | 0.1256 | 5.55:1 | 3.0:1 |
| `accentHover` | `#015593` | 0.0861 | 7.16:1 |  |
| `animated` | `#96712e` | 0.1849 | 4.15:1 |  |
| `success` | `#447e5c` | 0.1692 | 4.44:1 | 3.0:1 |
| `warning` | `#ac8539` | 0.2584 | 3.16:1 | 3.0:1 |
| `error` | `#8b375b` | 0.0898 | 6.97:1 | 3.0:1 |
| `cacheDisk` | `#287786` | 0.1537 | 4.78:1 |  |

Curve ramp: `#157e81`, `#6e7d4d`, `#9b607b`, `#877c4c`.

Layer ladder, darkest to lightest. Minimum step 8 L\*.

| Layer | Value | Y | L\* | Step | Chroma | Against surface1 |
|---|---|---|---|---|---|---|
| `layer.solid` | `#46413c` | 0.0541 | 27.9 |  | 4.0 | 9.36:1 |
| `layer.precomp` | `#644f5b` | 0.0906 | 36.1 | +8.2 | 11.6 | 6.93:1 |
| `layer.sequence` | `#61687b` | 0.1387 | 44.0 | +8.0 | 11.6 | 5.16:1 |
| `layer.footage` | `#63818a` | 0.2019 | 52.0 | +8.0 | 11.9 | 3.87:1 |
| `layer.camera` | `#a28c7f` | 0.2797 | 59.9 | +7.8 | 11.7 | 2.95:1 |
| `layer.text` | `#afa590` | 0.3804 | 68.0 | +8.2 | 12.2 | 2.26:1 |

Accent chroma 39.9 against a highest layer chroma of 12.2.

Role ladder: error L\* 35.9, success L\* 48.2, warning L\* 57.9. Accent to success: 115 degrees of hue, 6.1 L\*.

| Simulated | Closest layer pair | Closest role pair | Accent to success | Accent to nearest layer |
|---|---|---|---|---|
| Protanopia | 9.2 | 30.4 | 54.0 | 25.9 |
| Deuteranopia | 7.9 | 13.2 | 54.4 | 31.7 |
| Tritanopia | 10.0 | 28.5 | 10.6 | 13.8 |

## Neon

The cyber one, and the most saturated pair in the set. Near-black surfaces with a trace of violet, an electric magenta accent, cyan for the animated wells and the cache, and roles that glow: hot red, acid green, acid yellow. The light variant is not the neon washed out. It is a bright white page with saturated ink: the same magenta at a lightness that holds better than 5:1 on the panel, and roles that stay at full chroma while they come down the lightness scale. The layer hues are the one quiet thing in the pair, held under a third of the accent chroma so the timeline still reads as bars and not as a sign. The tight figure is deuteranopia on light, where the magenta accent and the pink precomp bar both go blue-grey and are 12.7 apart; the sequence bar was moved to a teal for that reason, since a violet one sat at 7.

### Neon dark

| Token | Value | Y | Against surface1 | Floor |
|---|---|---|---|---|
| `surface0` | `#1a1a1f` | 0.0106 | 1.10:1 |  |
| `surface1` | `#222228` | 0.0164 | ground |  |
| `surface2` | `#2a2b30` | 0.0243 | 1.12:1 |  |
| `surface3` | `#333339` | 0.0337 | 1.26:1 |  |
| `surface4` | `#3e3e44` | 0.0489 | 1.49:1 |  |
| `viewerSurround` | `#1c1c1c` | 0.0116 | neutral, R = G = B |  |
| `textPrimary` | `#ededf5` | 0.8517 | 13.58:1 | 7.0:1 |
| `textSecondary` | `#ceced6` | 0.6212 | 10.11:1 | 7.0:1 |
| `textMuted` | `#a0a0a7` | 0.3541 | 6.09:1 | 4.5:1 |
| `textDisabled` | `#717178` | 0.1668 | 3.27:1 | 3.0:1 |
| `hairline` | `#30313a` | 0.0313 | 1.22:1 |  |
| `hairlineStrong` | `#787983` | 0.1931 | 3.66:1 | 3.0:1 |
| `accent` | `#ff62df` | 0.3532 | 6.08:1 | 3.0:1 |
| `accentHover` | `#ff74f1` | 0.4010 | 6.80:1 |  |
| `animated` | `#23e7f3` | 0.6398 | 10.39:1 |  |
| `success` | `#52ce60` | 0.4678 | 7.80:1 | 3.0:1 |
| `warning` | `#ede92d` | 0.7647 | 12.27:1 | 3.0:1 |
| `error` | `#f22c73` | 0.2192 | 4.06:1 | 3.0:1 |
| `cacheDisk` | `#638ef0` | 0.2829 | 5.02:1 |  |

Curve ramp: `#09dddb`, `#a1d57d`, `#ff9eca`, `#ebd07a`.

Layer ladder, darkest to lightest. Minimum step 8 L\*.

| Layer | Value | Y | L\* | Step | Chroma | Against surface1 |
|---|---|---|---|---|---|---|
| `layer.solid` | `#514f55` | 0.0800 | 34.0 |  | 3.9 | 1.96:1 |
| `layer.precomp` | `#7c5777` | 0.1243 | 41.9 | +7.9 | 24.2 | 2.63:1 |
| `layer.sequence` | `#48826f` | 0.1849 | 50.1 | +8.2 | 24.2 | 3.54:1 |
| `layer.footage` | `#4b96a0` | 0.2585 | 57.9 | +7.8 | 23.8 | 4.65:1 |
| `layer.camera` | `#c2987c` | 0.3538 | 66.0 | +8.2 | 23.9 | 6.08:1 |
| `layer.text` | `#bfb78a` | 0.4678 | 74.0 | +8.0 | 24.3 | 7.80:1 |

Accent chroma 80.5 against a highest layer chroma of 24.3.

Role ladder: error L\* 53.9, success L\* 74.0, warning L\* 90.1. Accent to success: 167 degrees of hue, 8.1 L\*.

| Simulated | Closest layer pair | Closest role pair | Accent to success | Accent to nearest layer |
|---|---|---|---|---|
| Protanopia | 11.1 | 37.1 | 101.7 | 37.3 |
| Deuteranopia | 7.4 | 24.9 | 67.7 | 18.5 |
| Tritanopia | 12.6 | 56.9 | 103.6 | 37.5 |

### Neon light

| Token | Value | Y | Against surface1 | Floor |
|---|---|---|---|---|
| `surface0` | `#e5e5ec` | 0.7875 | 1.19:1 |  |
| `surface1` | `#f9f9fc` | 0.9492 | ground |  |
| `surface2` | `#efeff4` | 0.8662 | 1.09:1 |  |
| `surface3` | `#fcfcfe` | 0.9747 | 1.03:1 |  |
| `surface4` | `#dfdfe7` | 0.7423 | 1.26:1 |  |
| `viewerSurround` | `#a8a8a8` | 0.3916 | neutral, R = G = B |  |
| `textPrimary` | `#242533` | 0.0194 | 14.40:1 | 7.0:1 |
| `textSecondary` | `#454655` | 0.0630 | 8.84:1 | 7.0:1 |
| `textMuted` | `#6c6d7e` | 0.1563 | 4.84:1 | 4.5:1 |
| `textDisabled` | `#88899a` | 0.2546 | 3.28:1 | 3.0:1 |
| `hairline` | `#d6d6e2` | 0.6788 | 1.37:1 |  |
| `hairlineStrong` | `#858590` | 0.2378 | 3.47:1 | 3.0:1 |
| `accent` | `#c000a4` | 0.1389 | 5.29:1 | 3.0:1 |
| `accentHover` | `#ae0092` | 0.1107 | 6.22:1 |  |
| `animated` | `#a56200` | 0.1673 | 4.60:1 |  |
| `success` | `#008729` | 0.1749 | 4.44:1 | 3.0:1 |
| `warning` | `#9f8f00` | 0.2702 | 3.12:1 | 3.0:1 |
| `error` | `#ab004a` | 0.0915 | 7.06:1 | 3.0:1 |
| `cacheDisk` | `#256acf` | 0.1521 | 4.94:1 |  |

Curve ramp: `#028584`, `#518a2c`, `#c33e85`, `#978011`.

Layer ladder, darkest to lightest. Minimum step 8 L\*.

| Layer | Value | Y | L\* | Step | Chroma | Against surface1 |
|---|---|---|---|---|---|---|
| `layer.solid` | `#3e3d43` | 0.0477 | 26.1 |  | 4.0 | 10.23:1 |
| `layer.precomp` | `#6a4365` | 0.0802 | 34.0 | +8.0 | 26.2 | 7.68:1 |
| `layer.sequence` | `#2e6e5b` | 0.1249 | 42.0 | +8.0 | 25.9 | 5.71:1 |
| `layer.footage` | `#28828c` | 0.1831 | 49.9 | +7.9 | 26.0 | 4.29:1 |
| `layer.camera` | `#ae8265` | 0.2590 | 57.9 | +8.1 | 25.9 | 3.23:1 |
| `layer.text` | `#a9a172` | 0.3514 | 65.9 | +7.9 | 26.0 | 2.49:1 |

Accent chroma 83.3 against a highest layer chroma of 26.2.

Role ladder: error L\* 36.3, success L\* 48.9, warning L\* 59.0. Accent to success: 167 degrees of hue, 4.8 L\*.

| Simulated | Closest layer pair | Closest role pair | Accent to success | Accent to nearest layer |
|---|---|---|---|---|
| Protanopia | 11.3 | 14.3 | 100.0 | 33.5 |
| Deuteranopia | 7.2 | 21.2 | 64.4 | 12.7 |
| Tritanopia | 10.4 | 50.8 | 101.5 | 43.6 |

## Canopy

Deep forest greens, moss and bark, saturated but calm. The surfaces are green-black like wet bark, the text is the pale of birch, and the timeline hues are lichen, heather and a streak of sky. A green theme has one problem: the accent and the success lamp fight for the same word. It is resolved by splitting the two greens across the wheel and the ladder. The accent is a pale lichen yellow-green and the success lamp is a darker, bluer green, 50 degrees of hue and 16 L* apart on dark, 50 degrees and 9 L* on light, past the 40 degrees the rule asks for, and at least 31 dE apart under every deficiency. The accent fills buttons and draws the playhead; success only ever outlines a badge or fills a cache bar. The tight figure is tritanopia, where the lichen accent and the straw text bar both go pink-grey and sit 12.0 apart on dark and 12.7 on light; the accent was lifted two points of lightness to get there.

### Canopy dark

| Token | Value | Y | Against surface1 | Floor |
|---|---|---|---|---|
| `surface0` | `#161c17` | 0.0106 | 1.09:1 |  |
| `surface1` | `#1e241f` | 0.0164 | ground |  |
| `surface2` | `#262d27` | 0.0244 | 1.12:1 |  |
| `surface3` | `#2e3530` | 0.0334 | 1.26:1 |  |
| `surface4` | `#39413b` | 0.0497 | 1.50:1 |  |
| `viewerSurround` | `#1c1c1c` | 0.0116 | neutral, R = G = B |  |
| `textPrimary` | `#e8f0ea` | 0.8542 | 13.62:1 | 7.0:1 |
| `textSecondary` | `#c9d1cb` | 0.6233 | 10.15:1 | 7.0:1 |
| `textMuted` | `#9ba29d` | 0.3524 | 6.06:1 | 4.5:1 |
| `textDisabled` | `#6d746e` | 0.1687 | 3.29:1 | 3.0:1 |
| `hairline` | `#2a342c` | 0.0313 | 1.23:1 |  |
| `hairlineStrong` | `#717c73` | 0.1916 | 3.64:1 | 3.0:1 |
| `accent` | `#c2ce75` | 0.5690 | 9.33:1 | 3.0:1 |
| `accentHover` | `#d4e087` | 0.6906 | 11.16:1 |  |
| `animated` | `#e3bb77` | 0.5320 | 8.77:1 |  |
| `success` | `#57ab86` | 0.3287 | 5.71:1 | 3.0:1 |
| `warning` | `#f8d377` | 0.6788 | 10.98:1 | 3.0:1 |
| `error` | `#ca5661` | 0.2008 | 3.78:1 | 3.0:1 |
| `cacheDisk` | `#45a0c1` | 0.3026 | 5.31:1 |  |

Curve ramp: `#6ed0d3`, `#adca96`, `#eca8c6`, `#dbcc94`.

Layer ladder, darkest to lightest. Minimum step 8 L\*.

| Layer | Value | Y | L\* | Step | Chroma | Against surface1 |
|---|---|---|---|---|---|---|
| `layer.solid` | `#474d48` | 0.0712 | 32.1 |  | 4.1 | 1.83:1 |
| `layer.precomp` | `#705766` | 0.1122 | 39.9 | +7.9 | 13.8 | 2.44:1 |
| `layer.sequence` | `#647389` | 0.1678 | 48.0 | +8.0 | 13.9 | 3.28:1 |
| `layer.footage` | `#668d94` | 0.2401 | 56.1 | +8.1 | 14.2 | 4.37:1 |
| `layer.camera` | `#b29588` | 0.3274 | 63.9 | +7.8 | 14.0 | 5.69:1 |
| `layer.text` | `#bdaf97` | 0.4371 | 72.0 | +8.1 | 14.2 | 7.34:1 |

Accent chroma 46.2 against a highest layer chroma of 14.2.

Role ladder: error L\* 51.9, success L\* 64.1, warning L\* 85.9. Accent to success: 50 degrees of hue, 16.1 L\*.

| Simulated | Closest layer pair | Closest role pair | Accent to success | Accent to nearest layer |
|---|---|---|---|---|
| Protanopia | 9.7 | 21.9 | 33.4 | 33.8 |
| Deuteranopia | 8.2 | 15.3 | 37.2 | 27.9 |
| Tritanopia | 10.7 | 52.9 | 38.5 | 12.0 |

### Canopy light

| Token | Value | Y | Against surface1 | Floor |
|---|---|---|---|---|
| `surface0` | `#e1e7e0` | 0.7854 | 1.20:1 |  |
| `surface1` | `#f8faf8` | 0.9510 | ground |  |
| `surface2` | `#edf0ec` | 0.8638 | 1.10:1 |  |
| `surface3` | `#fbfdfb` | 0.9772 | 1.03:1 |  |
| `surface4` | `#dce1db` | 0.7418 | 1.26:1 |  |
| `viewerSurround` | `#a8a8a8` | 0.3916 | neutral, R = G = B |  |
| `textPrimary` | `#20281e` | 0.0192 | 14.47:1 | 7.0:1 |
| `textSecondary` | `#40493f` | 0.0621 | 8.93:1 | 7.0:1 |
| `textMuted` | `#677165` | 0.1563 | 4.85:1 | 4.5:1 |
| `textDisabled` | `#838d81` | 0.2546 | 3.29:1 | 3.0:1 |
| `hairline` | `#d1d9d0` | 0.6774 | 1.38:1 |  |
| `hairlineStrong` | `#81887f` | 0.2381 | 3.47:1 | 3.0:1 |
| `accent` | `#556500` | 0.1124 | 6.16:1 | 3.0:1 |
| `accentHover` | `#435300` | 0.0738 | 8.09:1 |  |
| `animated` | `#9a6f27` | 0.1839 | 4.28:1 |  |
| `success` | `#1d845e` | 0.1757 | 4.43:1 | 3.0:1 |
| `warning` | `#b08829` | 0.2700 | 3.13:1 | 3.0:1 |
| `error` | `#a32e3f` | 0.1010 | 6.63:1 | 3.0:1 |
| `cacheDisk` | `#007795` | 0.1536 | 4.92:1 |  |

Curve ramp: `#007f82`, `#5f8047`, `#a55a7d`, `#897c3d`.

Layer ladder, darkest to lightest. Minimum step 8 L\*.

| Layer | Value | Y | L\* | Step | Chroma | Against surface1 |
|---|---|---|---|---|---|---|
| `layer.solid` | `#3e443f` | 0.0552 | 28.2 |  | 4.2 | 9.52:1 |
| `layer.precomp` | `#694c5e` | 0.0898 | 35.9 | +7.8 | 16.5 | 7.16:1 |
| `layer.sequence` | `#586982` | 0.1379 | 43.9 | +8.0 | 15.9 | 5.33:1 |
| `layer.footage` | `#56838b` | 0.2008 | 51.9 | +8.0 | 16.1 | 3.99:1 |
| `layer.camera` | `#aa8a7c` | 0.2818 | 60.0 | +8.1 | 15.7 | 3.02:1 |
| `layer.text` | `#b4a489` | 0.3806 | 68.1 | +8.0 | 16.2 | 2.32:1 |

Accent chroma 49.9 against a highest layer chroma of 16.5.

Role ladder: error L\* 38.0, success L\* 49.0, warning L\* 59.0. Accent to success: 50 degrees of hue, 9.0 L\*.

| Simulated | Closest layer pair | Closest role pair | Accent to success | Accent to nearest layer |
|---|---|---|---|---|
| Protanopia | 10.2 | 22.3 | 31.8 | 41.5 |
| Deuteranopia | 8.3 | 14.8 | 34.5 | 36.7 |
| Tritanopia | 11.0 | 42.2 | 31.7 | 12.7 |

## Slate

Monochrome, for someone who finds a themed editor distracting. There is no hue in the chrome at all: every surface, every text step and both hairlines are neutral greys, R = G = B, and the script fails if one of them drifts. Colour appears only where it carries meaning: a plain blue accent, the three role lamps, the layer family and the curve ramp. It is not Chalk. Chalk is the high-contrast pair with near-black under near-white and a ten-point layer ladder; Slate sits on the same lightness ladder as the other pairs, with ordinary contrast, and only the tint removed. The tight figure is the one the rules describe, a blue accent under tritanopia: 12.1 from success on dark, and on light 12.2 from success and 11.0 from the footage bar. The accent was moved from 250 to 265 degrees on light to keep the red it needs there.

### Slate dark

| Token | Value | Y | Against surface1 | Floor |
|---|---|---|---|---|
| `surface0` | `#1a1a1a` | 0.0103 | 1.11:1 |  |
| `surface1` | `#232323` | 0.0168 | ground |  |
| `surface2` | `#2b2b2b` | 0.0242 | 1.11:1 |  |
| `surface3` | `#343434` | 0.0343 | 1.26:1 |  |
| `surface4` | `#3f3f3f` | 0.0497 | 1.49:1 |  |
| `viewerSurround` | `#1c1c1c` | 0.0116 | neutral, R = G = B |  |
| `textPrimary` | `#eeeeee` | 0.8550 | 13.55:1 | 7.0:1 |
| `textSecondary` | `#cfcfcf` | 0.6240 | 10.09:1 | 7.0:1 |
| `textMuted` | `#a0a0a0` | 0.3515 | 6.01:1 | 4.5:1 |
| `textDisabled` | `#727272` | 0.1683 | 3.27:1 | 3.0:1 |
| `hairline` | `#313131` | 0.0307 | 1.21:1 |  |
| `hairlineStrong` | `#797979` | 0.1912 | 3.61:1 | 3.0:1 |
| `accent` | `#33beef` | 0.4376 | 7.30:1 | 3.0:1 |
| `accentHover` | `#45d0ff` | 0.5360 | 8.77:1 |  |
| `animated` | `#e3bc6f` | 0.5345 | 8.75:1 |  |
| `success` | `#54b882` | 0.3778 | 6.40:1 | 3.0:1 |
| `warning` | `#f5d477` | 0.6783 | 10.90:1 | 3.0:1 |
| `error` | `#d25586` | 0.2192 | 4.03:1 | 3.0:1 |
| `cacheDisk` | `#669bca` | 0.3053 | 5.32:1 |  |

Curve ramp: `#62d8db`, `#afd195`, `#eca7d2`, `#dbcc94`.

Layer ladder, darkest to lightest. Minimum step 8 L\*.

| Layer | Value | Y | L\* | Step | Chroma | Against surface1 |
|---|---|---|---|---|---|---|
| `layer.solid` | `#4b4b4b` | 0.0704 | 31.9 |  | 0.0 | 1.80:1 |
| `layer.precomp` | `#645b6d` | 0.1130 | 40.1 | +8.2 | 11.6 | 2.44:1 |
| `layer.sequence` | `#677386` | 0.1687 | 48.1 | +8.0 | 11.9 | 3.27:1 |
| `layer.footage` | `#6c8c93` | 0.2405 | 56.1 | +8.0 | 12.0 | 4.35:1 |
| `layer.camera` | `#ad9789` | 0.3282 | 64.0 | +7.9 | 11.9 | 5.66:1 |
| `layer.text` | `#b8b09b` | 0.4361 | 72.0 | +7.9 | 11.8 | 7.28:1 |

Accent chroma 40.0 against a highest layer chroma of 12.0.

Role ladder: error L\* 53.9, success L\* 67.9, warning L\* 85.9. Accent to success: 84 degrees of hue, 4.2 L\*.

| Simulated | Closest layer pair | Closest role pair | Accent to success | Accent to nearest layer |
|---|---|---|---|---|
| Protanopia | 8.8 | 30.0 | 55.1 | 30.6 |
| Deuteranopia | 7.7 | 14.8 | 54.4 | 33.3 |
| Tritanopia | 9.0 | 51.9 | 12.1 | 31.2 |

### Slate light

| Token | Value | Y | Against surface1 | Floor |
|---|---|---|---|---|
| `surface0` | `#e5e5e5` | 0.7835 | 1.20:1 |  |
| `surface1` | `#f9f9f9` | 0.9473 | ground |  |
| `surface2` | `#efefef` | 0.8632 | 1.09:1 |  |
| `surface3` | `#fcfcfc` | 0.9734 | 1.03:1 |  |
| `surface4` | `#dfdfdf` | 0.7379 | 1.27:1 |  |
| `viewerSurround` | `#a8a8a8` | 0.3916 | neutral, R = G = B |  |
| `textPrimary` | `#262626` | 0.0194 | 14.37:1 | 7.0:1 |
| `textSecondary` | `#474747` | 0.0630 | 8.82:1 | 7.0:1 |
| `textMuted` | `#6e6e6e` | 0.1559 | 4.84:1 | 4.5:1 |
| `textDisabled` | `#8a8a8a` | 0.2542 | 3.28:1 | 3.0:1 |
| `hairline` | `#d7d7d7` | 0.6795 | 1.37:1 |  |
| `hairlineStrong` | `#868686` | 0.2384 | 3.46:1 | 3.0:1 |
| `accent` | `#006da9` | 0.1380 | 5.30:1 | 3.0:1 |
| `accentHover` | `#005b97` | 0.0972 | 6.78:1 |  |
| `animated` | `#9b6f22` | 0.1845 | 4.25:1 |  |
| `success` | `#0f8351` | 0.1693 | 4.55:1 | 3.0:1 |
| `warning` | `#ad8928` | 0.2693 | 3.12:1 | 3.0:1 |
| `error` | `#a81e5f` | 0.1008 | 6.61:1 | 3.0:1 |
| `cacheDisk` | `#39719e` | 0.1515 | 4.95:1 |  |

Curve ramp: `#007f82`, `#5b8141`, `#a4598a`, `#897c3d`.

Layer ladder, darkest to lightest. Minimum step 8 L\*.

| Layer | Value | Y | L\* | Step | Chroma | Against surface1 |
|---|---|---|---|---|---|---|
| `layer.solid` | `#424242` | 0.0545 | 28.0 |  | 0.0 | 9.55:1 |
| `layer.precomp` | `#5b5166` | 0.0907 | 36.1 | +8.1 | 13.7 | 7.09:1 |
| `layer.sequence` | `#5b697f` | 0.1386 | 44.0 | +7.9 | 14.0 | 5.29:1 |
| `layer.footage` | `#5d828b` | 0.2016 | 52.0 | +8.0 | 13.9 | 3.96:1 |
| `layer.camera` | `#a58c7c` | 0.2821 | 60.1 | +8.1 | 13.9 | 3.00:1 |
| `layer.text` | `#aea58d` | 0.3783 | 67.9 | +7.8 | 13.6 | 2.33:1 |

Accent chroma 39.5 against a highest layer chroma of 14.0.

Role ladder: error L\* 38.0, success L\* 48.2, warning L\* 58.9. Accent to success: 109 degrees of hue, 4.2 L\*.

| Simulated | Closest layer pair | Closest role pair | Accent to success | Accent to nearest layer |
|---|---|---|---|---|
| Protanopia | 8.9 | 32.9 | 61.0 | 23.0 |
| Deuteranopia | 7.5 | 12.6 | 58.8 | 28.3 |
| Tritanopia | 9.1 | 45.4 | 12.2 | 11.0 |

## Nocturne

Deep indigo and violet night with a warm accent, moody rather than technical. The surfaces carry half again the tint of Glacier and sit at 292 degrees, violet, where Glacier sits at 260, blue; the text is lavender-white; the accent is apricot, a warm point on a cold ground, where Glacier is cyan on blue-black. The animated wells are straw, the roles are rose, a sea green and a pale gold, and the timeline is violet and heather with one copper bar for cameras. Nothing in it is tight: the warm accent is far from every cold bar under every deficiency.

### Nocturne dark

| Token | Value | Y | Against surface1 | Floor |
|---|---|---|---|---|
| `surface0` | `#191a24` | 0.0107 | 1.10:1 |  |
| `surface1` | `#22222d` | 0.0167 | ground |  |
| `surface2` | `#2a2a36` | 0.0241 | 1.11:1 |  |
| `surface3` | `#33333e` | 0.0342 | 1.26:1 |  |
| `surface4` | `#3e3e4a` | 0.0496 | 1.49:1 |  |
| `viewerSurround` | `#1c1c1c` | 0.0116 | neutral, R = G = B |  |
| `textPrimary` | `#ededf7` | 0.8529 | 13.53:1 | 7.0:1 |
| `textSecondary` | `#ceced7` | 0.6217 | 10.07:1 | 7.0:1 |
| `textMuted` | `#a0a0a9` | 0.3548 | 6.07:1 | 4.5:1 |
| `textDisabled` | `#71717a` | 0.1673 | 3.26:1 | 3.0:1 |
| `hairline` | `#30303f` | 0.0310 | 1.21:1 |  |
| `hairlineStrong` | `#787889` | 0.1923 | 3.63:1 | 3.0:1 |
| `accent` | `#ff9560` | 0.4360 | 7.28:1 | 3.0:1 |
| `accentHover` | `#ffa772` | 0.5011 | 8.26:1 |  |
| `animated` | `#d8cd89` | 0.6007 | 9.75:1 |  |
| `success` | `#5bb883` | 0.3814 | 6.46:1 | 3.0:1 |
| `warning` | `#f6d782` | 0.6981 | 11.21:1 | 3.0:1 |
| `error` | `#d3538b` | 0.2190 | 4.03:1 | 3.0:1 |
| `cacheDisk` | `#4f9eca` | 0.3038 | 5.30:1 |  |

Curve ramp: `#6ed0d3`, `#adca96`, `#e6a9cf`, `#dbcc94`.

Layer ladder, darkest to lightest. Minimum step 8 L\*.

| Layer | Value | Y | L\* | Step | Chroma | Against surface1 |
|---|---|---|---|---|---|---|
| `layer.solid` | `#4a4b51` | 0.0708 | 32.0 |  | 3.8 | 1.81:1 |
| `layer.precomp` | `#6b586b` | 0.1117 | 39.9 | +7.9 | 14.0 | 2.42:1 |
| `layer.sequence` | `#607489` | 0.1678 | 48.0 | +8.1 | 14.1 | 3.26:1 |
| `layer.footage` | `#678c95` | 0.2381 | 55.9 | +7.9 | 13.8 | 4.32:1 |
| `layer.camera` | `#b4948c` | 0.3278 | 64.0 | +8.1 | 13.8 | 5.66:1 |
| `layer.text` | `#b9b097` | 0.4360 | 72.0 | +8.0 | 14.0 | 7.28:1 |

Accent chroma 57.0 against a highest layer chroma of 14.1.

Role ladder: error L\* 53.9, success L\* 68.1, warning L\* 86.9. Accent to success: 103 degrees of hue, 3.8 L\*.

| Simulated | Closest layer pair | Closest role pair | Accent to success | Accent to nearest layer |
|---|---|---|---|---|
| Protanopia | 9.3 | 26.9 | 15.2 | 25.5 |
| Deuteranopia | 8.6 | 17.4 | 33.4 | 33.8 |
| Tritanopia | 10.1 | 54.1 | 86.8 | 34.5 |

### Nocturne light

| Token | Value | Y | Against surface1 | Floor |
|---|---|---|---|---|
| `surface0` | `#e4e5ee` | 0.7871 | 1.19:1 |  |
| `surface1` | `#f9f9fc` | 0.9492 | ground |  |
| `surface2` | `#eeeff5` | 0.8650 | 1.09:1 |  |
| `surface3` | `#fcfcfe` | 0.9747 | 1.03:1 |  |
| `surface4` | `#dedfe9` | 0.7419 | 1.26:1 |  |
| `viewerSurround` | `#a8a8a8` | 0.3916 | neutral, R = G = B |  |
| `textPrimary` | `#222536` | 0.0193 | 14.42:1 | 7.0:1 |
| `textSecondary` | `#434658` | 0.0628 | 8.86:1 | 7.0:1 |
| `textMuted` | `#6b6d81` | 0.1565 | 4.84:1 | 4.5:1 |
| `textDisabled` | `#87899e` | 0.2551 | 3.27:1 | 3.0:1 |
| `hairline` | `#d5d6e5` | 0.6790 | 1.37:1 |  |
| `hairlineStrong` | `#848593` | 0.2379 | 3.47:1 | 3.0:1 |
| `accent` | `#b54d1b` | 0.1521 | 4.94:1 | 3.0:1 |
| `accentHover` | `#a33b09` | 0.1093 | 6.27:1 |  |
| `animated` | `#927229` | 0.1831 | 4.29:1 |  |
| `success` | `#21824c` | 0.1681 | 4.58:1 | 3.0:1 |
| `warning` | `#af882e` | 0.2692 | 3.13:1 | 3.0:1 |
| `error` | `#a71f63` | 0.1010 | 6.62:1 | 3.0:1 |
| `cacheDisk` | `#11759f` | 0.1534 | 4.91:1 |  |

Curve ramp: `#007f82`, `#5f8047`, `#9f5c87`, `#897c3d`.

Layer ladder, darkest to lightest. Minimum step 8 L\*.

| Layer | Value | Y | L\* | Step | Chroma | Against surface1 |
|---|---|---|---|---|---|---|
| `layer.solid` | `#414248` | 0.0549 | 28.1 |  | 3.9 | 9.53:1 |
| `layer.precomp` | `#634e64` | 0.0902 | 36.0 | +7.9 | 16.2 | 7.13:1 |
| `layer.sequence` | `#526b82` | 0.1392 | 44.1 | +8.1 | 16.0 | 5.28:1 |
| `layer.footage` | `#57838d` | 0.2018 | 52.0 | +7.9 | 16.0 | 3.97:1 |
| `layer.camera` | `#ad897f` | 0.2831 | 60.2 | +8.1 | 16.1 | 3.00:1 |
| `layer.text` | `#afa589` | 0.3783 | 67.9 | +7.7 | 15.9 | 2.33:1 |

Accent chroma 62.0 against a highest layer chroma of 16.2.

Role ladder: error L\* 38.0, success L\* 48.0, warning L\* 58.9. Accent to success: 102 degrees of hue, 2.1 L\*.

| Simulated | Closest layer pair | Closest role pair | Accent to success | Accent to nearest layer |
|---|---|---|---|---|
| Protanopia | 10.4 | 27.2 | 17.8 | 37.8 |
| Deuteranopia | 8.2 | 17.7 | 31.6 | 38.8 |
| Tritanopia | 10.4 | 43.5 | 95.2 | 45.9 |

## Painted materials

Three pairs from one visual world, a warm painted one of lacquered wood and brass, candlelight, parchment, gemstones and gold leaf. Each is named and described by its materials. They meet every rule the general pairs meet and depend on nothing outside this section, so the group can be dropped whole.

### Tavern

Lacquered dark wood, aged brass, candle amber and cream parchment. It is not Hearth. Hearth is matte and domestic, a kitchen table in brown-charcoal with a terracotta accent and cupboard roles. Tavern is varnished: its surfaces carry nearly twice the chroma and sit sixteen degrees redder on the wheel, the text is a warm cream rather than an off-white, the accent is a candle amber twelve degrees yellower and a quarter more saturated than terracotta, the hairlines are brass, and the roles are wine, bottle green and a pale gold. The light variant is parchment with dark-wood ink. Nothing in it is tight; the closest figure is the wine and the bottle green under deuteranopia, at 18 and above.

#### Tavern dark

| Token | Value | Y | Against surface1 | Floor |
|---|---|---|---|---|
| `surface0` | `#261710` | 0.0106 | 1.10:1 |  |
| `surface1` | `#2f1f19` | 0.0165 | ground |  |
| `surface2` | `#382721` | 0.0240 | 1.11:1 |  |
| `surface3` | `#413029` | 0.0340 | 1.26:1 |  |
| `surface4` | `#4d3b34` | 0.0495 | 1.50:1 |  |
| `viewerSurround` | `#1c1c1c` | 0.0116 | neutral, R = G = B |  |
| `textPrimary` | `#ffe9e1` | 0.8497 | 13.52:1 | 7.0:1 |
| `textSecondary` | `#e1cac1` | 0.6210 | 10.08:1 | 7.0:1 |
| `textMuted` | `#b29c93` | 0.3535 | 6.06:1 | 4.5:1 |
| `textDisabled` | `#826e66` | 0.1686 | 3.28:1 | 3.0:1 |
| `hairline` | `#432c23` | 0.0312 | 1.22:1 |  |
| `hairlineStrong` | `#907368` | 0.1919 | 3.64:1 | 3.0:1 |
| `accent` | `#f79245` | 0.4076 | 6.88:1 | 3.0:1 |
| `accentHover` | `#ffa457` | 0.4850 | 8.04:1 |  |
| `animated` | `#e9d082` | 0.6405 | 10.38:1 |  |
| `success` | `#66b07b` | 0.3531 | 6.06:1 | 3.0:1 |
| `warning` | `#f2d97a` | 0.6991 | 11.26:1 | 3.0:1 |
| `error` | `#c24d80` | 0.1834 | 3.51:1 | 3.0:1 |
| `cacheDisk` | `#45a0c1` | 0.3026 | 5.30:1 |  |

Curve ramp: `#70d0ce`, `#b2c993`, `#e9a2bc`, `#d5c68f`.

Layer ladder, darkest to lightest. Minimum step 8 L\*.

| Layer | Value | Y | L\* | Step | Chroma | Against surface1 |
|---|---|---|---|---|---|---|
| `layer.solid` | `#514a47` | 0.0710 | 32.0 |  | 3.7 | 1.82:1 |
| `layer.precomp` | `#735663` | 0.1120 | 39.9 | +7.9 | 14.5 | 2.43:1 |
| `layer.sequence` | `#6f7088` | 0.1675 | 47.9 | +8.0 | 14.2 | 3.27:1 |
| `layer.footage` | `#668d94` | 0.2401 | 56.1 | +8.2 | 14.2 | 4.36:1 |
| `layer.camera` | `#b5938f` | 0.3267 | 63.9 | +7.8 | 13.9 | 5.66:1 |
| `layer.text` | `#b9b097` | 0.4360 | 72.0 | +8.1 | 14.0 | 7.30:1 |

Accent chroma 64.1 against a highest layer chroma of 14.5.

Role ladder: error L\* 49.9, success L\* 66.0, warning L\* 87.0. Accent to success: 90 degrees of hue, 4.0 L\*.

| Simulated | Closest layer pair | Closest role pair | Accent to success | Accent to nearest layer |
|---|---|---|---|---|
| Protanopia | 10.4 | 31.7 | 26.8 | 38.4 |
| Deuteranopia | 8.4 | 20.4 | 40.9 | 43.4 |
| Tritanopia | 13.4 | 51.7 | 82.2 | 36.2 |

#### Tavern light

| Token | Value | Y | Against surface1 | Floor |
|---|---|---|---|---|
| `surface0` | `#f4e3cf` | 0.7868 | 1.19:1 |  |
| `surface1` | `#fef8f2` | 0.9462 | ground |  |
| `surface2` | `#f9eee0` | 0.8667 | 1.09:1 |  |
| `surface3` | `#fffcf8` | 0.9766 | 1.03:1 |  |
| `surface4` | `#eeddc9` | 0.7411 | 1.26:1 |  |
| `viewerSurround` | `#a8a8a8` | 0.3916 | neutral, R = G = B |  |
| `textPrimary` | `#302412` | 0.0193 | 14.37:1 | 7.0:1 |
| `textSecondary` | `#534432` | 0.0620 | 8.89:1 | 7.0:1 |
| `textMuted` | `#7c6c57` | 0.1570 | 4.81:1 | 4.5:1 |
| `textDisabled` | `#998872` | 0.2560 | 3.26:1 | 3.0:1 |
| `hairline` | `#e5d5c1` | 0.6810 | 1.36:1 |  |
| `hairlineStrong` | `#938472` | 0.2392 | 3.44:1 | 3.0:1 |
| `accent` | `#ad540b` | 0.1525 | 4.92:1 | 3.0:1 |
| `accentHover` | `#9b4200` | 0.1086 | 6.28:1 |  |
| `animated` | `#957125` | 0.1833 | 4.27:1 |  |
| `success` | `#32814d` | 0.1691 | 4.55:1 | 3.0:1 |
| `warning` | `#b08824` | 0.2697 | 3.12:1 | 3.0:1 |
| `error` | `#9f1d5e` | 0.0906 | 7.09:1 | 3.0:1 |
| `cacheDisk` | `#007795` | 0.1536 | 4.89:1 |  |

Curve ramp: `#00807e`, `#647f43`, `#a85978`, `#897c3d`.

Layer ladder, darkest to lightest. Minimum step 8 L\*.

| Layer | Value | Y | L\* | Step | Chroma | Against surface1 |
|---|---|---|---|---|---|---|
| `layer.solid` | `#48413d` | 0.0550 | 28.1 |  | 4.1 | 9.49:1 |
| `layer.precomp` | `#6b4c5a` | 0.0903 | 36.0 | +7.9 | 15.8 | 7.10:1 |
| `layer.sequence` | `#646681` | 0.1380 | 43.9 | +7.9 | 16.2 | 5.30:1 |
| `layer.footage` | `#56838b` | 0.2008 | 51.9 | +8.0 | 16.1 | 3.97:1 |
| `layer.camera` | `#ae8883` | 0.2825 | 60.1 | +8.2 | 16.0 | 3.00:1 |
| `layer.text` | `#afa589` | 0.3783 | 67.9 | +7.8 | 15.9 | 2.33:1 |

Accent chroma 61.8 against a highest layer chroma of 16.2.

Role ladder: error L\* 36.1, success L\* 48.2, warning L\* 58.9. Accent to success: 92 degrees of hue, 2.2 L\*.

| Simulated | Closest layer pair | Closest role pair | Accent to success | Accent to nearest layer |
|---|---|---|---|---|
| Protanopia | 11.2 | 32.7 | 23.4 | 41.6 |
| Deuteranopia | 8.6 | 18.5 | 35.1 | 42.1 |
| Tritanopia | 14.3 | 41.8 | 85.6 | 38.6 |

### Arcane

Cold spell light: deep indigo, frost cyan, silver, and a violet accent. The surfaces are the bluest in the set and among the most tinted, the text is silver rather than white, the animated wells are frost, and the accent is a saturated violet, bluer and stronger than Mallow's lavender and on a ground that is indigo rather than mauve. The roles are rose, a frost green and a pale gold. It is kept apart from Nocturne by the accent: Nocturne warms its night with apricot, Arcane keeps everything cold. The tight figure is tritanopia on light, where the violet accent and the violet precomp bar keep only their red and sit 12.6 apart.

#### Arcane dark

| Token | Value | Y | Against surface1 | Floor |
|---|---|---|---|---|
| `surface0` | `#111b28` | 0.0106 | 1.09:1 |  |
| `surface1` | `#192330` | 0.0162 | ground |  |
| `surface2` | `#212c39` | 0.0242 | 1.12:1 |  |
| `surface3` | `#2a3442` | 0.0334 | 1.26:1 |  |
| `surface4` | `#353f4e` | 0.0486 | 1.49:1 |  |
| `viewerSurround` | `#1c1c1c` | 0.0116 | neutral, R = G = B |  |
| `textPrimary` | `#eaeef5` | 0.8523 | 13.63:1 | 7.0:1 |
| `textSecondary` | `#cbcfd6` | 0.6218 | 10.14:1 | 7.0:1 |
| `textMuted` | `#9da1a7` | 0.3545 | 6.11:1 | 4.5:1 |
| `textDisabled` | `#6e7279` | 0.1673 | 3.28:1 | 3.0:1 |
| `hairline` | `#253243` | 0.0308 | 1.22:1 |  |
| `hairlineStrong` | `#6d7b8e` | 0.1937 | 3.68:1 | 3.0:1 |
| `accent` | `#afa6ff` | 0.4361 | 7.34:1 | 3.0:1 |
| `accentHover` | `#c1b8ff` | 0.5284 | 8.73:1 |  |
| `animated` | `#7de1ed` | 0.6432 | 10.47:1 |  |
| `success` | `#56b88b` | 0.3812 | 6.51:1 | 3.0:1 |
| `warning` | `#f3d886` | 0.6989 | 11.31:1 | 3.0:1 |
| `error` | `#d05593` | 0.2201 | 4.08:1 | 3.0:1 |
| `cacheDisk` | `#509fc5` | 0.3053 | 5.37:1 |  |

Curve ramp: `#76d6d4`, `#b2cf9b`, `#eaa8ca`, `#dbcc94`.

Layer ladder, darkest to lightest. Minimum step 8 L\*.

| Layer | Value | Y | L\* | Step | Chroma | Against surface1 |
|---|---|---|---|---|---|---|
| `layer.solid` | `#484c51` | 0.0714 | 32.1 |  | 3.6 | 1.83:1 |
| `layer.precomp` | `#69596d` | 0.1125 | 40.0 | +7.9 | 13.9 | 2.45:1 |
| `layer.sequence` | `#597687` | 0.1683 | 48.0 | +8.0 | 14.0 | 3.30:1 |
| `layer.footage` | `#668d8e` | 0.2383 | 55.9 | +7.9 | 14.0 | 4.35:1 |
| `layer.camera` | `#b09686` | 0.3276 | 64.0 | +8.1 | 14.0 | 5.70:1 |
| `layer.text` | `#b9b097` | 0.4360 | 72.0 | +8.0 | 14.0 | 7.34:1 |

Accent chroma 48.7 against a highest layer chroma of 14.0.

Role ladder: error L\* 54.0, success L\* 68.1, warning L\* 86.9. Accent to success: 138 degrees of hue, 3.9 L\*.

| Simulated | Closest layer pair | Closest role pair | Accent to success | Accent to nearest layer |
|---|---|---|---|---|
| Protanopia | 9.2 | 29.1 | 62.2 | 39.0 |
| Deuteranopia | 7.7 | 17.7 | 55.0 | 38.3 |
| Tritanopia | 8.3 | 52.1 | 33.2 | 19.4 |

#### Arcane light

| Token | Value | Y | Against surface1 | Floor |
|---|---|---|---|---|
| `surface0` | `#dfe6ec` | 0.7834 | 1.20:1 |  |
| `surface1` | `#f7fafc` | 0.9517 | ground |  |
| `surface2` | `#ebf0f4` | 0.8651 | 1.09:1 |  |
| `surface3` | `#fbfcfe` | 0.9729 | 1.02:1 |  |
| `surface4` | `#d9e1e7` | 0.7437 | 1.26:1 |  |
| `viewerSurround` | `#a8a8a8` | 0.3916 | neutral, R = G = B |  |
| `textPrimary` | `#0b2938` | 0.0194 | 14.43:1 | 7.0:1 |
| `textSecondary` | `#2f4a5b` | 0.0626 | 8.90:1 | 7.0:1 |
| `textMuted` | `#577284` | 0.1573 | 4.83:1 | 4.5:1 |
| `textDisabled` | `#738ea1` | 0.2556 | 3.28:1 | 3.0:1 |
| `hairline` | `#cad9e5` | 0.6784 | 1.38:1 |  |
| `hairlineStrong` | `#7a8893` | 0.2385 | 3.47:1 | 3.0:1 |
| `accent` | `#6059c8` | 0.1380 | 5.33:1 | 3.0:1 |
| `accentHover` | `#4e47b6` | 0.0950 | 6.91:1 |  |
| `animated` | `#987736` | 0.2014 | 3.99:1 |  |
| `success` | `#118255` | 0.1674 | 4.61:1 | 3.0:1 |
| `warning` | `#a98b31` | 0.2712 | 3.12:1 | 3.0:1 |
| `error` | `#9e1b66` | 0.0901 | 7.15:1 | 3.0:1 |
| `cacheDisk` | `#16759a` | 0.1523 | 4.95:1 |  |

Curve ramp: `#00807e`, `#5f8047`, `#a35b82`, `#897c3d`.

Layer ladder, darkest to lightest. Minimum step 8 L\*.

| Layer | Value | Y | L\* | Step | Chroma | Against surface1 |
|---|---|---|---|---|---|---|
| `layer.solid` | `#3f4248` | 0.0542 | 27.9 |  | 4.1 | 9.61:1 |
| `layer.precomp` | `#614f65` | 0.0907 | 36.1 | +8.2 | 15.5 | 7.12:1 |
| `layer.sequence` | `#4b6c80` | 0.1378 | 43.9 | +7.8 | 16.1 | 5.33:1 |
| `layer.footage` | `#578485` | 0.2022 | 52.1 | +8.2 | 16.0 | 3.97:1 |
| `layer.camera` | `#a88b79` | 0.2817 | 60.0 | +8.0 | 16.0 | 3.02:1 |
| `layer.text` | `#afa589` | 0.3783 | 67.9 | +7.9 | 15.9 | 2.34:1 |

Accent chroma 66.0 against a highest layer chroma of 16.1.

Role ladder: error L\* 36.0, success L\* 47.9, warning L\* 59.1. Accent to success: 142 degrees of hue, 4.0 L\*.

| Simulated | Closest layer pair | Closest role pair | Accent to success | Accent to nearest layer |
|---|---|---|---|---|
| Protanopia | 9.3 | 31.9 | 79.6 | 44.5 |
| Deuteranopia | 7.5 | 18.4 | 72.4 | 43.1 |
| Tritanopia | 8.7 | 45.5 | 25.0 | 12.6 |

### Gilt

Near-black with gold leaf, an amber accent, and a deep plum shadow. The most ornamental pair in the set: the surfaces are black with a plum cast that deepens toward surface4, the strong hairline is gold leaf rather than grey, the text is ivory, and the accent is amber. The roles are ruby, a dark leaf green and a pale gold. The light variant is ivory with plum-black ink and a dark-gold hairline. The tight figure is protanopia on dark, where the plum solid bar and the rose precomp bar above it are 7.9 apart; the precomp hue was moved from 355 to 345 degrees, since at 355 the light variant fell to 6.9.

#### Gilt dark

| Token | Value | Y | Against surface1 | Floor |
|---|---|---|---|---|
| `surface0` | `#20181f` | 0.0106 | 1.09:1 |  |
| `surface1` | `#282027` | 0.0163 | ground |  |
| `surface2` | `#31292f` | 0.0244 | 1.12:1 |  |
| `surface3` | `#393138` | 0.0335 | 1.26:1 |  |
| `surface4` | `#453c43` | 0.0490 | 1.49:1 |  |
| `viewerSurround` | `#1c1c1c` | 0.0116 | neutral, R = G = B |  |
| `textPrimary` | `#f6ebf4` | 0.8554 | 13.65:1 | 7.0:1 |
| `textSecondary` | `#d6ccd4` | 0.6224 | 10.14:1 | 7.0:1 |
| `textMuted` | `#a89ea6` | 0.3553 | 6.11:1 | 4.5:1 |
| `textDisabled` | `#796f77` | 0.1677 | 3.28:1 | 3.0:1 |
| `hairline` | `#392e37` | 0.0310 | 1.22:1 |  |
| `hairlineStrong` | `#837680` | 0.1934 | 3.67:1 | 3.0:1 |
| `accent` | `#f0a840` | 0.4690 | 7.83:1 | 3.0:1 |
| `accentHover` | `#ffba52` | 0.5699 | 9.35:1 |  |
| `animated` | `#e2ca84` | 0.6008 | 9.81:1 |  |
| `success` | `#61ab76` | 0.3298 | 5.73:1 | 3.0:1 |
| `warning` | `#efde7d` | 0.7207 | 11.62:1 | 3.0:1 |
| `error` | `#c14474` | 0.1673 | 3.28:1 | 3.0:1 |
| `cacheDisk` | `#4aa0c3` | 0.3054 | 5.36:1 |  |

Curve ramp: `#70d0ce`, `#b2c993`, `#e6a2c0`, `#d5c68f`.

Layer ladder, darkest to lightest. Minimum step 8 L\*.

| Layer | Value | Y | L\* | Step | Chroma | Against surface1 |
|---|---|---|---|---|---|---|
| `layer.solid` | `#4f4a4e` | 0.0711 | 32.1 |  | 3.4 | 1.83:1 |
| `layer.precomp` | `#725764` | 0.1131 | 40.1 | +8.0 | 13.8 | 2.46:1 |
| `layer.sequence` | `#6f7088` | 0.1675 | 47.9 | +7.8 | 14.2 | 3.28:1 |
| `layer.footage` | `#698c98` | 0.2403 | 56.1 | +8.2 | 13.8 | 4.38:1 |
| `layer.camera` | `#b3948a` | 0.3260 | 63.8 | +7.7 | 13.9 | 5.67:1 |
| `layer.text` | `#b6b197` | 0.4362 | 72.0 | +8.1 | 14.2 | 7.33:1 |

Accent chroma 64.0 against a highest layer chroma of 14.2.

Role ladder: error L\* 47.9, success L\* 64.1, warning L\* 88.0. Accent to success: 76 degrees of hue, 10.0 L\*.

| Simulated | Closest layer pair | Closest role pair | Accent to success | Accent to nearest layer |
|---|---|---|---|---|
| Protanopia | 7.9 | 33.6 | 36.9 | 47.7 |
| Deuteranopia | 7.7 | 16.7 | 46.4 | 48.2 |
| Tritanopia | 14.1 | 50.7 | 71.7 | 27.7 |

#### Gilt light

| Token | Value | Y | Against surface1 | Floor |
|---|---|---|---|---|
| `surface0` | `#ede4d6` | 0.7835 | 1.20:1 |  |
| `surface1` | `#fcf9f4` | 0.9498 | ground |  |
| `surface2` | `#f5eee5` | 0.8622 | 1.10:1 |  |
| `surface3` | `#fefcf8` | 0.9747 | 1.02:1 |  |
| `surface4` | `#e8dfd0` | 0.7449 | 1.26:1 |  |
| `viewerSurround` | `#a8a8a8` | 0.3916 | neutral, R = G = B |  |
| `textPrimary` | `#2c2518` | 0.0192 | 14.44:1 | 7.0:1 |
| `textSecondary` | `#4e4637` | 0.0628 | 8.87:1 | 7.0:1 |
| `textMuted` | `#766d5e` | 0.1560 | 4.85:1 | 4.5:1 |
| `textDisabled` | `#938979` | 0.2547 | 3.28:1 | 3.0:1 |
| `hairline` | `#e1d6c4` | 0.6809 | 1.37:1 |  |
| `hairlineStrong` | `#8f8575` | 0.2390 | 3.46:1 | 3.0:1 |
| `accent` | `#a16000` | 0.1594 | 4.77:1 | 3.0:1 |
| `accentHover` | `#8f4e00` | 0.1129 | 6.14:1 |  |
| `animated` | `#93792d` | 0.2007 | 3.99:1 |  |
| `success` | `#32814d` | 0.1691 | 4.56:1 | 3.0:1 |
| `warning` | `#a88c22` | 0.2720 | 3.11:1 | 3.0:1 |
| `error` | `#a31557` | 0.0901 | 7.14:1 | 3.0:1 |
| `cacheDisk` | `#057698` | 0.1526 | 4.94:1 |  |

Curve ramp: `#00807e`, `#647f43`, `#a55a7d`, `#897c3d`.

Layer ladder, darkest to lightest. Minimum step 8 L\*.

| Layer | Value | Y | L\* | Step | Chroma | Against surface1 |
|---|---|---|---|---|---|---|
| `layer.solid` | `#464045` | 0.0540 | 27.8 |  | 4.2 | 9.61:1 |
| `layer.precomp` | `#6a4c5c` | 0.0901 | 36.0 | +8.2 | 16.1 | 7.14:1 |
| `layer.sequence` | `#646681` | 0.1380 | 43.9 | +7.9 | 16.2 | 5.32:1 |
| `layer.footage` | `#598290` | 0.2010 | 52.0 | +8.0 | 15.9 | 3.98:1 |
| `layer.camera` | `#ac897d` | 0.2814 | 60.0 | +8.1 | 16.2 | 3.02:1 |
| `layer.text` | `#aca689` | 0.3785 | 67.9 | +7.9 | 16.1 | 2.33:1 |

Accent chroma 58.7 against a highest layer chroma of 16.2.

Role ladder: error L\* 36.0, success L\* 48.2, warning L\* 59.2. Accent to success: 80 degrees of hue, 1.3 L\*.

| Simulated | Closest layer pair | Closest role pair | Accent to success | Accent to nearest layer |
|---|---|---|---|---|
| Protanopia | 8.3 | 34.7 | 24.5 | 41.7 |
| Deuteranopia | 7.5 | 14.0 | 36.1 | 43.1 |
| Tritanopia | 15.1 | 48.9 | 72.8 | 27.2 |

## What it costs to add them

`LumitColorScheme` in `flutter_ui/lib/theme/theme.dart` gains twenty-six values, and each gets a factory shaped exactly like `LumitTheme.gruvboxDark()`: the same fields and nothing new, since every scheme maps onto the roles that already exist. The `label`, `mode` and `build` switches grow twenty-six arms each. The thirteen names are proper nouns and can stay literal in `label` as Gruvbox and Catppuccin do, or go through `app_en.arb` if the picker is ever translated. The Rust `Theme` in `crates/lumit-ui` carries the same seven schemes and would take the same twenty-six factories to stay in step. The contrast check CI already runs against the theme struct will cover the new ones with no change. Section 11.1 of `docs/15-DESIGN.md` lists the named schemes and takes one line per pair. Nothing in widget code moves. The four painted-material pairs are one block in the script and one section here; dropping them is deleting that block and those four entries.

