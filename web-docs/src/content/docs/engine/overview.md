---
title: How Lumit is built
description: The engine and interface.
sidebar:
  order: 1
---

This section explains how Lumit works below the user interface, with information on maths 
operations, and exactly how the program renders a frame. None of this knowledge is 
required to use Lumit, but is useful if you want to know why Lumit behaves in specific 
ways, or if you want to help contribute to Lumit.

## Front and backend

| Half | Language | What it does |
| --- | --- | --- |
| **The backend** | Rust | The backend engine performs all calculations, along with writing to memory and disk. |
| **The frontend** | Flutter | The frontend draws the GUI, where possible all adjustments and logic are calculated on the backend, then reflected in the GUI. |

The frontend aims to hold no information of its own that hasn't been passed from the backend.
It should only display values, and forward commands to the backend.
This allows the backend to work without any GUI required, which can help with allowing exporting,
and testing, without requiring the program to be opened.

## The crates making up the backend

The Rust backend is split into separate libraries: GPU, media, audio, cache, document model, 
etc. 

## GPU first

The rendering pipeline works on the GPU, using D3D12, Vulkan, or Metal depending on 
your platform. Lumit has no software fallback.

## Related

- [The render pipeline](/engine/render-pipeline/)
- [Time and precision](/engine/time/)
- [Caching](/engine/cache/)
