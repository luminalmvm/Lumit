---
title: The render pipeline
description: How a composition becomes a frame.
sidebar:
  order: 2
---

## From composition to frame

A composition can be viewed as a document, with each layer explaining how 
to render something specific. Before rendering, Lumit compiles it into an
**DAG**, or **evaluation graph**, which calculates everything required to 
render that particular frame.

For instance, indentical layers or duplicates (such as duplicating a 
precomp) only need to be rendered once, and the output is reused. This 
helps reduce the work required when rendering expensive frames.

The DAG is completely internal. As a user you never interact or see it.

## Content hashing

Each item of work is hashed, which is altered by anything that affects the
rendered result: the source layer, transform property changes, effects.

If two items hash as the same value, then they will be visually identical,
which allows us to reuse the [cached](/engine/cache/) rendered texture.

## Order of operation

Within a composition, it renders from the bottom layer to the top. If a
layer references another layer that is above it in the composition, or 
that wouldn't be otherwise rendered (e.g. invisible), then it will stop
rendering the current layer until the referenced layer is rendered, then
continue.

For each layer, it renders in the following order: decodes source frame, 
retime, flow, masks, effects, transform, and then uses the blend mode to 
composite into the existing frame.

The effects and masks are composited from the top item to the bottom as 
they appear in the effects column or in the mask layer row. For instance
if you apply a glow effect above (meaning before) an exposure effect, it
will have a very different affect on the output, than if you reverse the
order.

## Related

- [Caching](/engine/cache/)
- [Time and precision](/engine/time/)
