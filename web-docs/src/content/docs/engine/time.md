---
title: Time and precision
description: Storing time as exact fractions.
sidebar:
  order: 3
---

## The issue with float frame rates

If a footage or a composition is 29.97 frames per second, then one frame is exactly 
1001/30000 of a second long, which is equivalent to a non-terminating decimal value. 
If we attempt to store this as a float value then we won't store the length of a frame 
perfectly, and in longer compositions, the footage will begin to drift apart from the 
frame it should actually be displaying at a specific timecode.

## What Lumit does

To resolve this we instead store the length of time to display a frame as the exact
fraction, as demonstrated above. This requires two integer values, the numerator and
denominator. As per the example above, we can now store the frame time for a 29.97 fps
composition exactly, where numerator = 1001, and denominator = 30000. For simpler 
frame rates, such as 60 fps, this can be calculated as 1/60.

## References to time

"Time" itself can refer to several different parts of a project, which is 
why internally they are separated into the following:

| Time type | Measured from |
| --- | --- |
| **Source time** | The start of the decoded media file. |
| **Clip time** | The start of an individual clip, inside a Sequence layer. |
| **Layer time** | The start of a layer within a composition. |
| **Comp time** | The start of the composition. |

## Related

- [Retiming and speed](/use/retime/)
- [The render pipeline](/engine/render-pipeline/)
