# The resource governor: one ledger for the memory a render spends

**Built: `crates/lumit-budget`, wired through `lumit-gpu`, `lumit-cache`,
`lumit-render` and the bridge's status readout.**

[13-PERFORMANCE-RULES.md](../13-PERFORMANCE-RULES.md) §3 specifies the governor
and §4 the degradation ladder it feeds; this note is the binding *how* — where
the budgets come from, what is registered against them, which rungs of the
ladder the renderer takes on its own, and what is deliberately left alone.

**In plain terms:** every frame-sized thing a render makes — a decoded picture,
an effect's intermediate, a shutter sample, a measured flow field — is tens or
hundreds of megabytes, and each on its own is a perfectly reasonable size. The
problem was that nothing added them up. A project is a file, and a comp at 8K
with forty effects and a thirty-two-sample shutter is something a person can
build by accident on a Tuesday; it is also something a person can build on
purpose. So there is now one account, everything frame-sized is registered
against it, and when it runs short the renderer gives up the things nobody will
miss before the machine gives up the device.

---

## 1. The ledger

`lumit_budget::Ledger` holds two tiers — `Vram` and `Ram` — because they run out
separately and for different reasons. Asking is `reserve`, which answers with a
`Reservation` or with a refusal naming what was wanted and what was free.

The `Reservation` *is* the accounting. Holding it is what makes the memory
yours; dropping it is what gives it back; there is no release to call, so there
is no path that forgets to, including the paths that unwind through an early
return or a failure half way down. A reservation that outlives its allocation
only makes the ledger pessimistic. One that dies first makes it wrong, which is
why every one of them is kept in the same struct as the thing it paid for.

Grants are a compare-and-swap rather than a fetch-add, so two workers racing for
the last hundred megabytes get one grant and one refusal instead of two grants.
Releases saturate, so a double release cannot drive the used figure below zero
and conjure memory that was never there.

The crate is arithmetic and nothing else: `forbid(unsafe_code)`, no I/O, no
platform calls. It counts; it does not allocate, and it is handed the machine's
figures rather than asking for them.

## 2. Where the budgets come from

Per docs/13 §3: **70% of what the card reports, 60% of physical RAM.** Getting
these from the machine rather than from a constant is what makes every reading
above them mean anything, and it is wrong in both directions if it is not:

* A 24 GB card told it had a 2 GB fallback declares itself full with 22 GB free,
  and steps the ladder down on the fastest machine in the building.
* A 2 GB card told the same hands out every byte it has and then some, which is
  the driver reset the whole design exists to prevent.

One is merely slow. The other is the crash.

Two layers supply the figures, each answering for what it can honestly see:

| Layer | Sets | From |
|---|---|---|
| `GpuContext::headless` | `Vram` | `lumit_gpu::video_memory_bytes()` — Metal's recommended working-set size, or the largest device-local Vulkan heap, with the adapter still in hand |
| the render worker | both | the bridge's `video_memory_bytes` (DXGI on Windows, the above elsewhere) and `system_memory_bytes` |

A graphics context has no business making a system call about how much RAM the
machine has, which is why host memory is not set there.

**Unified memory** is the case neither layer could decide alone, because it
needs both figures: when the card draws from system memory — every Apple Silicon
Mac, any integrated adapter — the two tiers are one pool, and 70% of it plus 60%
of it is 130% of the same memory. The card's share is capped at 40% of the
machine there. A cap rather than a replacement: Metal's recommended working-set
size is already a share of the unified memory and is the better of the two
numbers, so the cap only bites where the card reported something optimistic, or
reported nothing at all.

A platform that will not answer says **0**, and 0 falls back to
`DEFAULT_VRAM_BUDGET` / `DEFAULT_RAM_BUDGET` — per tier, so a machine that knows
its RAM and not its card keeps the figure it does have.

## 3. What is registered

### 3.1 The frame, at the one boundary that can still say no

A texture cannot be refused half way through a pass; the pass would draw nothing
rather than something smaller. So the decision is made once, before any of it
starts: `GpuContext::try_begin_frame(estimate)` reserves the frame's expected
**peak**, and `Realiser::realise_region` is where the estimate is worked out.

Peak, not total, and the difference is large. Layers composite one at a time
into one accumulator and the effect walk ping-pongs through the frame's texture
pool, so a hundred-layer comp peaks at a handful of pictures rather than a
hundred. What grows with the project is the layers that stage through a
comp-sized intermediate of their own — an adjustment layer, which needs
everything below it composited first, and a motion-blurred one, which holds an
accumulator across its shutter.

Every work texture is then charged as it is made, so what the ledger says is
held is what is held. `charge_vram` **cannot refuse**, deliberately: the caller
has nowhere to go without the texture. It records the overdraft instead, which
is what keeps the pressure reading honest, the next frame's grant with it, and
the readout truthful.

### 3.2 The caches, at their one admission point

`lumit_cache::ByteLru` is the store every byte-budgeted tier already shares, so
the account lives there: a store is registered once with a ledger and a tier,
and from then on tells it what it holds after every change — an insert, the
eviction that insert caused, a replacement, a lowered budget, being emptied. All
of those already converge on one resync, so there is no way to change the
contents and forget to say so. A store nobody registers is exactly what it was.

Registered today: the card's finished frames and the per-effect intermediates
(`Vram`), the decoded-source-frame cache and the measured-flow cache (`Ram`).

### 3.3 The frame in flight, which nothing was counting

A decoded comp frame is the renderer's largest allocation that is neither a
cache entry nor a frame's own texture. A comp of twenty 4K layers, each with
four temporal neighbours, a measured flow field and eight shutter moments,
reaches for tens of gigabytes of ordinary memory before anything is drawn.

It is made on the decode thread, sent down a channel, and dropped by whoever
finished drawing it. No scope spans its life, so it carries its own reservation
and gives it back when it goes — including when it goes because a newer frame
superseded it, which is the case a hand-written release would never have covered.

### 3.4 The hand-off

An intermediate the effect cache files is the one texture that outlives the
frame that made it. Its charge moves with it: the frame puts the bytes down as
the store takes them up (`GpuContext::hand_off_vram`), so one texture is one
charge at every moment, in both directions. An overdraft is settled before a
reservation is, because those bytes were never granted and giving them back
would credit the ledger for memory it never lent.

## 4. Which rungs the renderer takes

docs/13 §4 orders the ladder. The renderer takes the first two on its own and
no others, and the line between them is not a matter of taste:

**Rungs 1 and 2 are invisible.** Pausing the intermediate cache fill and giving
back the cold half of it change nothing about the picture — only whether it is
made again or read back. That is what makes them steps a renderer may take on a
memory reading at all.

1. **Pause cache fill.** The effect walk reads the pressure and stops filing
   while it lasts. Lookups are untouched: what is already held is free to read.
   It takes itself back, so there is no flag to clear.
2. **Give back the cold half.** A refused frame trims the intermediate store and
   the decode stores by their own cost-aware eviction order, then asks again. A
   pin is never dropped — the decode planner skipped work on the strength of one.

Beside them sits one more thing a frame at the ceiling does, invisible for the
same reason: it **gives up its batching**. Handing a texture back to the frame's
pool is an engine-side promise, but the driver only stops holding what a
recorded command refers to once that command has been submitted — so a
forty-effect stack recorded into one batch keeps every transient in the driver
until the frame closes. At `Pressure::Full` the walk flushes effect by effect,
the same trade a profiled frame already makes and the same one the lens flare
makes between its own batches. It costs round trips, which is time; it bounds
what the driver holds, which is what has run out.

That all three are invisible is a claim worth being able to falsify, so it is a
test: the same stack rendered on an empty card and on a full one, compared byte
for byte.

**Rungs 3 to 7 change what is drawn** — the preview resolution tier, tiling, the
flow-to-blend swap, the CPU fallback, the calm banner — so they belong to the
caller, which reads the denial count and steps down deliberately. Export never
takes rungs 3 and 5 at all: under pressure export slows down, it never changes
output.

**A frame refused twice is rendered anyway.** There is nothing honest to return
instead, and a black frame is a worse answer than a slow one. What the refusal
buys is a truthful readout and a governor that says no earlier next time.

The same rule holds everywhere a refusal arrives after the memory already
exists — a store resyncing, a decoded frame being weighed. The reservation falls
short, the denial is counted, and the pressure that reads from is what makes the
*next* decode trim before it starts. Obeying it would mean dropping a picture
something is still holding a handle to, or handing back a frame with layers
missing, and changing the picture on a memory reading is the one thing the
ladder never does.

## 5. The readout

A ladder nobody can see stepping is a bug in itself (docs/13 §4: "silent
degradation is a bug"). Settings → Preview and cache shows both tiers: what is
held against the ceiling, which rung that puts the renderer on in the user's own
terms ("Nearly full — pausing cache fill"), the session peak, and the count of
requests turned away.

That last line is the one that earns the section. A tier sitting under its
budget with refusals climbing is turning work away, which is a different fault
from one that is merely full and is invisible in every other number on the page.

Unlike the memory report beside it, this is **not** debug-only. A preview that
quietly stopped caching because the card filled is exactly the case that reaches
a bug tracker as "Lumit went slow for no reason".

## 6. Deliberately not done

* **Live budget renegotiation.** docs/13 §3 wants the governor to subscribe to
  DXGI video-memory budget-change notifications and to OS memory pressure, and
  shrink its budgets while running. The ceilings are read once, at start-up.
  `Ledger::set_budget` is the seam that will take the new figure: it is already
  live, already safe to call at any moment, and already confiscates nothing
  (the reservations that exist are memory that exists — it refuses everything
  new until enough has been given back).
* **Pool aliasing and graph-derived lifetimes** (docs/13 §3, last bullet). The
  frame's work-texture pool recycles by shape within a frame; deriving per-node
  lifetimes from the compiled graph's refcounts waits on the evaluation graph
  owning the pixel pass.
* **Rungs 3–7.** Owned by the caller, as above.
* **Open decoders are counted, not weighed.** What a decoder holds is FFmpeg's
  and the driver's business, and a made-up number of bytes would be worse than
  an honest count.
