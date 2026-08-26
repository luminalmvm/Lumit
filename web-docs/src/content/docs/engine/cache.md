---
title: Caching
description: How Lumit reduces render work.
sidebar:
  order: 4
---

Rendering a frame can become expensive. After a frame is rendered, this value is stored
to reuse later. This helps improve realtime playback, and reduces the time required to
preview work that you've already done.

## Cache hashing

Cached items are unique and match to the hash key calculated when rendering the frame.
If a frame's hash matches a cache entry this can be sent to the GPU to be displayed,
preventing the need to re-render a frame.

## Cache tiers

There are three tiers to the cache. The cache works on a first in, first out approach.
When a cache is filled, it removes the oldest frame, and the frame is demoted to the
next cache. If the frame is demoted out of the disk cache, it is deleted.

Caches can be cleared by clicking on the cache in Lumit's bottom strip, or within settings.

:::note
In practice, we actually try to perform a calculation of a frame's render time, against
how old it is, to find what frame to remove.
:::

### VRAM

The VRAM cache stores cached frames directly on the GPU. These allow for the fastest
loading of files to display, and the most recently cached frames are stored on it.
When Lumit closes, these cached frames are cleared.

### RAM

The RAM cache remains relatively fast to load a preview frame. 
When Lumit closes, these cached frames are cleared.

### Disk

The disk cache is the slowest, but remains faster for intensive compositions.
When Lumit closes, these cached frames do not clear, meaning they can fill up the RAM
and VRAM caches upon loading. While using Lumit, the other caches will send cached 
frames to the disk cache to keep them across sessions.

## Related

- [Preview and playback](/use/preview/)
- [The render pipeline](/engine/render-pipeline/)
