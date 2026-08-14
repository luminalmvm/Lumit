# Rust, taught from Lumit's code

For a developer who knows Delphi well and has met Java, Python and a little C. Every
example is real code from this repository. Line numbers drift. The path and the shown
code are the anchor.

## The Delphi translation table

| Delphi | Rust | Note |
|---|---|---|
| `unit` | module (`mod`) / crate | A crate is a compilation unit, like a package |
| `interface`/`implementation` sections | `pub` on each item | Visibility is per item, not per section |
| `type TFoo = class` | `struct Foo` + `impl Foo` | Data and methods are declared separately |
| `interface` (COM-style) | `trait` | A capability contract |
| `record` | `struct` | Rust structs are values by default |
| Variant records / `case` | `enum` with data | Far safer: the compiler checks every branch |
| `try..finally` | `Drop` | Cleanup is automatic at scope end |
| `raise`/`except` | `Result<T, E>` + `?` | Errors are return values, not control flow |
| `nil` | `Option<T>` | There is no null |
| `TObject` reference counting | `Arc<T>` / `Rc<T>` | Explicit, and `Arc` is thread-safe |
| `const`/`var` parameters | `&T` / `&mut T` | Borrowing, checked at compile time |
| `TThread` + `TCriticalSection` | threads + `Mutex`/`RwLock`/channels | The compiler refuses unsound sharing |

Three ideas have no Delphi equivalent and cause most early confusion: **ownership**,
**the borrow checker**, and **exhaustive matching**. The rest is familiar.

## 1. Ownership and moves

Every value has exactly one owner. Assigning or passing it *moves* it unless the type
is `Copy` or you borrow with `&`. When the owner goes out of scope, Rust drops the
value. There is no garbage collector and no manual `Free`.

This is why the codebase clones deliberately, and says so. The edit path clones the
whole document on purpose:

```rust
// crates/lumit-core/src/store.rs — `DocumentStore::commit`
    pub fn commit(&self, op: Op) -> Result<Arc<Document>, OpError> {
        let mut journal = self.journal.lock();
        let mut doc = Document::clone(&self.snapshot());
        let inverse = apply(&mut doc, &op)?;

        let observed = op.clone();
        journal.undo.push(JournalEntry { op, inverse });
        journal.redo.clear();
```

Read the signature. `&self` borrows the store immutably. `op: Op` takes ownership of
the command. `Document::clone` makes an independent copy so readers holding the old
one are unaffected. `&mut doc` lends the copy to `apply` for modification.

`Arc<T>` is a shared, reference-counted handle. It is the closest thing to a Delphi
interface reference, but thread-safe. Cloning an `Arc` copies a pointer, not the
data. The frame-key code makes that point explicitly:

```rust
// crates/lumit-eval/src/lib.rs — `comp_frame_key`
    let mut visited = Vec::new();
    let mut h = blake3::Hasher::new();
    // Takes the document as an `&Arc` because an expression context needs an
    // owned handle on it: the caller's Arc is shared, so naming a frame clones
    // a pointer, never the project. (A deep clone here once cost hundreds of
    // MB per cache-bar refinement turn on a large project.)
    feed_comp(&mut h, doc, comp, t, quality, stamper, &mut visited)?;
```

## 2. Borrowing

`&T` is a shared borrow (many allowed, read-only). `&mut T` is an exclusive borrow
(one at a time, read-write). The compiler enforces that you never hold both at once.
That single rule eliminates data races and iterator invalidation.

In practice you rarely think about it, because functions take what they need:

```rust
// crates/lumit-media/src/encode.rs — `pick_first_working`
pub fn pick_first_working<'a>(
    candidates: &[&'a str],
    mut works: impl FnMut(&'a str) -> bool,
) -> Option<&'a str> {
    candidates.iter().copied().find(|name| works(name))
}
```

`'a` is a lifetime: it says the returned string reference lives as long as the input
slice's contents. You will read many lifetimes and write few. The compiler infers
most. Note also `impl FnMut(...) -> bool`: a closure parameter. This function is the
encoder ladder. It is testable without any hardware, because the "does it work" test
is injected.

## 3. Option: there is no null

```rust
// crates/lumit-render/src/source.rs — `impl SourceProbes for NoProbes`
impl SourceProbes for NoProbes {
    fn probe(&self, _item: Uuid) -> SourceProbe {
        SourceProbe::Unprobed
    }
}

impl SourceProbes for std::collections::HashMap<Uuid, SourceProbe> {
    fn probe(&self, item: Uuid) -> SourceProbe {
        self.get(&item).copied().unwrap_or(SourceProbe::Unprobed)
    }
}
```

`get` returns `Option<&SourceProbe>`. `.copied()` turns `Option<&T>` into
`Option<T>`. `.unwrap_or(x)` supplies a default. No nil check, no access violation.

The `?` operator propagates `None` early. This reads like a guard clause chain:

```rust
// crates/lumit-core/src/expression/layer.rs — `with_layer`
fn with_layer<T>(
    context: &NativeCallContext,
    this: &Layer,
    read: impl FnOnce(&Arc<ExpressionContext>, &model::Layer) -> T,
) -> Option<T> {
    let (comp_id, layer_id) = this.ids?;
    let context = ExpressionContext::from_call(context);
    let comp = context.document.comp(comp_id)?;
    let layer = comp.layers.iter().find(|l| l.id == layer_id)?;
    Some(read(&context, layer))
}
```

Each `?` means "if this is `None`, return `None` now". `<T>` is a generic: the caller
decides what the closure returns.

## 4. Result: errors are values

```rust
// crates/lumit-media/src/lib.rs — `MediaError`
#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("ffmpeg: {0}")]
    Ffmpeg(String),
    #[error("path is not valid unicode")]
    BadPath,
    #[error("no streams found")]
    NoStreams,
    #[error("index cache: {0}")]
    IndexCache(String),
```

`#[derive(...)]` generates code. Here `thiserror` writes the `Display` and `Error`
implementations from the `#[error(...)]` attributes. `#[from]` generates a conversion,
so `?` on an IO error automatically becomes a `MediaError::Io`.

At the bridge, errors map to sentences the status line can show:

```rust
// crates/lumit-bridge/src/api/project.rs — `ProjectReference::save`
    pub fn save(&self, path: String) -> Result<String, BridgeError> {
        let state = self.state()?;
        let mut state = state.write().map_err(|_| BridgeError::WriteFailed)?;

        let target = if path.trim().is_empty() {
            // Never saved and no path given: the caller has to pick one.
            state.path.clone().ok_or(BridgeError::NoProjectPath)?
        } else {
            std::path::PathBuf::from(path)
        };
```

Three idioms in six lines: `?` propagates, `.map_err(...)` converts one error type to
another, `.ok_or(...)` turns an `Option` into a `Result`. Note `if ... { } else { }`
is an *expression* that produces a value. This is like Delphi's `IfThen`, but for any
type.

**Lumit bans `unwrap`, `expect`, `panic!` and `unsafe` workspace-wide** (root
`Cargo.toml`). A failure degrades to a picture (identity, hold, empty frame), never
a crash.

## 5. Structs, enums, and exhaustive matching

An `enum` is a variant record whose tag the compiler tracks:

```rust
// crates/lumit-core/src/anim.rs — `Animation`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Animation {
    Static(f64),
    /// Sorted by time, unique times (enforced by the editing ops).
    Keyframed(Vec<Keyframe>),
    Expression(String),
}
```

`match` must handle every variant, or it does not compile:

```rust
// crates/lumit-core/src/anim.rs — `Property::value_at`
    pub fn value_at(&self, t: f64) -> f64 {
        match &self.animation {
            Animation::Static(v) => *v,
            Animation::Keyframed(keys) => evaluate(keys, t).unwrap_or(0.0),
            Animation::Expression(expression) => crate::expression::evaluate(expression, None),
        }
    }
```

This is the single most useful property in the codebase. Adding a variant makes the
compiler list every place that must change. It is why the strict glossary maps so
well to code. It is also why the bridge's `op_scope` match is safe: a new `Op` is a
compile error until it is handled.

Variants can carry structured data:

```rust
// crates/lumit-render/src/export.rs — `ExportEvent`
pub enum ExportEvent {
    /// Which encoder the ladder settled on ("NVENC", "software x264", …),
    /// sent once the file is open.
    Encoder(&'static str),
    Progress {
        frame: usize,
        total: usize,
    },
    Done(PathBuf),
    Failed(String),
}
```

## 6. Traits

A trait is an interface. Define it, implement it for any type, and accept it as a
parameter. Lumit uses traits to make untestable things testable:

```rust
// crates/lumit-eval/src/exec.rs — `CacheStore`
/// The rendered-frame cache, keyed by content hash (docs/06 §5.2). `get`
/// before any work; `put` after a completed render — including one that
/// turns out stale, because the work is already paid for (docs/06 §6.3).
pub trait CacheStore {
    fn get(&mut self, key: FrameKey) -> Option<FrameHandle>;
    fn put(&mut self, key: FrameKey, frame: FrameHandle);
}
```

`&mut dyn CacheStore` is a trait object. It gives dynamic dispatch, like a Delphi
interface variable. The executor takes three of them, so it unit-tests with fakes and
no GPU:

```rust
// crates/lumit-eval/src/exec.rs — `render_frame`
pub fn render_frame(
    graph: &EvalGraph,
    t: f64,
    key: Option<FrameKey>,
    source: &mut dyn FrameSource,
    kernels: &mut dyn KernelExecutor,
    cache: &mut dyn CacheStore,
    token: &EpochToken,
) -> Result<FrameHandle, ExecError> {
```

You also implement standard-library traits to gain behaviour. `Ord` gives sorting and
comparison:

```rust
// crates/lumit-core/src/time.rs — `impl Ord for Rational`
impl Ord for Rational {
    /// Cross-multiply in i128 — never in i64 (docs/impl/rational-time.md §2).
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let lhs = i128::from(self.num) * i128::from(other.den);
        let rhs = i128::from(other.num) * i128::from(self.den);
        lhs.cmp(&rhs)
    }
}
```

Operator overloading is a trait too:

```rust
// crates/lumit-core/src/fx/fft.rs — `impl std::ops::Add for Cx`
impl std::ops::Add for Cx {
    type Output = Cx;
    fn add(self, o: Cx) -> Cx {
        Cx::new(self.re + o.re, self.im + o.im)
    }
}
```

## 7. Newtypes: making wrong code not compile

A newtype wraps one value to give it a distinct type. Lumit's four timebases are
newtypes over `Rational`, generated by a macro:

```rust
// crates/lumit-core/src/time.rs — `Duration` and `timebase!`
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Duration(pub Rational);

macro_rules! timebase {
    ($(#[$doc:meta])* $T:ident) => {
        $(#[$doc])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash,
            Serialize, Deserialize,
        )]
        pub struct $T(pub Rational);
```

Deliberately, these types overload **no** arithmetic operators. You can only
`add_dur`, `sub_dur`, and `delta`. Adding a `CompTime` to a `SourceTime` is a compile
error. The bug class simply cannot be written.

`macro_rules!` is compile-time code generation, closer to a template than to a C
`#define`.

## 8. Closures and iterators

Closures are anonymous functions that can capture their environment. `move` transfers
ownership of what they capture. This is necessary when the closure outlives the
current scope:

```rust
// crates/lumit-bridge/src/api/state.rs — `LumitBridgeState::new_project`
        state.store.set_callback(Arc::new(move |c| {
            Self::handle_change_callback(c, id, &journal)
        }));

        PROJECTS
            .write()
            .map_err(|_| BridgeError::WriteFailed)?
            .insert(id, Arc::new(RwLock::new(state)));

        Ok(ProjectReference::new(id))
```

Iterator chains replace most loops. They are lazy and compile to the same code a
hand-written loop would:

```rust
// crates/lumit-core/src/model.rs — `MotionBlur::sample_offsets`
    pub fn sample_offsets(&self) -> Vec<f64> {
        if !self.enabled || self.samples < 2 {
            return Vec::new();
        }
        let n = self.samples.min(Self::MAX_SAMPLES);
        let open_frac = self.shutter_angle / 360.0;
        let phase_frac = self.shutter_phase / 360.0;
        (0..n)
            .map(|k| phase_frac + (f64::from(k) + 0.5) / f64::from(n) * open_frac)
            .collect()
    }
```

`zip` walks several collections together. This code builds the composite layer list
from five parallel vectors at once:

```rust
// crates/lumit-render/src/realise.rs — `Realiser::realise_segment`
        let comp_layers: Vec<lumit_gpu::CompositeLayer> = linear_textures
            .iter()
            .zip(layers)
            .zip(&matte_textures)
            .zip(&mask_textures)
            .zip(&mb_textures)
            .map(|((((texture, l), matte_tex), mask_tex), mb_tex)| {
```

## 9. Threads without fear

Rust's ownership rules extend to threads: the compiler refuses to share what is not
safe to share. Channels move values between threads:

```rust
// crates/lumit-render/src/diskio.rs — `spawn`
pub fn spawn() -> DiskIo {
    let (tx, rx) = std::sync::mpsc::channel::<Cmd>();
    let (loaded_tx, loaded) = std::sync::mpsc::channel();
    let known: Arc<Mutex<HashSet<u128>>> = Arc::default();
    let used_bytes: Arc<AtomicU64> = Arc::default();
    let pending: Arc<Mutex<ParkQueue>> = Arc::default();
    let known_worker = known.clone();
    let used_worker = used_bytes.clone();
    let pending_worker = pending.clone();
    std::thread::Builder::new()
        .name("nebula-disk".into())
        .spawn(move || {
```

Note the pattern: clone an `Arc` per thread, then `move` the clones into the closure.
Shared counters use atomics, with no lock at all:

```rust
// crates/lumit-eval/src/epoch.rs — `Epoch::bump` and `Epoch::token`
    /// Invalidate all outstanding tokens (playhead moved, stop pressed…).
    pub fn bump(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    /// Stamp a token for work scheduled now.
    pub fn token(&self) -> EpochToken {
        EpochToken {
            epoch: self.clone(),
            seen: self.0.load(Ordering::Relaxed),
        }
    }
```

**Lock scoping is the house rule.** A lock lasts for the shortest possible block, and
never spans an await, a GPU call, or a render. The idiom is a block that returns the
data you need:

```rust
// crates/lumit-bridge/src/api/worker_thread.rs — `play_one_frame`
            let (document, revision) = {
                let Ok(document) = state.project.state() else {
                    return;
                };
                let Ok(document) = document.read() else {
                    return;
                };
                (document.store.snapshot(), document.store.revision())
            };
```

`let ... else` is the guard clause. It binds on success, or takes the `else` branch,
which must diverge (`return`, `break`, `continue`). The guard drops at the closing
brace.

Real-time code takes this further. The audio callback never waits at all:

```rust
// crates/lumit-audio/src/lib.rs — `fill`
fn fill(shared: &Shared, out: &mut [f32], channels: usize) {
    out.fill(0.0);
    if !shared.playing.load(Ordering::Relaxed) {
        return;
    }
    let Some(guard) = shared.plan.try_read() else {
        return; // plan being swapped: one quiet buffer beats a glitch
    };
```

## 10. Drop: cleanup without `finally`

When a value goes out of scope, its `Drop` runs. This replaces `try..finally` and
makes RAII the default:

```rust
// crates/lumit-gpu/src/lib.rs — `impl Drop for EncoderGuard<'_>`
impl Drop for EncoderGuard<'_> {
    fn drop(&mut self) {
        // Only an owned encoder is submitted here. A batched one belongs to the
        // frame and is submitted once, by `end_frame`.
        if let Some(enc) = self.owned.take() {
            self.ctx.submit([enc.finish()]);
        }
    }
}
```

## 11. Serde: serialisation by derive

Most types serialise by adding one derive. Where a custom format matters, implement
the traits by hand:

```rust
// crates/lumit-core/src/time.rs — `impl Serialize for Rational`
impl Serialize for Rational {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        (self.num, self.den).serialize(s)
    }
}

impl<'de> Deserialize<'de> for Rational {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (num, den) = <(i64, i64)>::deserialize(d)?;
        Self::new(num, den).map_err(serde::de::Error::custom)
    }
}
```

A `Rational` writes as `[num, den]` and re-normalises on read. Attributes control the
rest: `#[serde(default)]`, `#[serde(skip_serializing_if = "...")]`, and
`#[serde(flatten)]` for the `extra` maps that let unknown fields round-trip.

## 12. Tests live beside the code

```rust
// crates/lumit-media/src/lib.rs — `cache_key_does_not_panic_on_a_short_hash`
    #[test]
    fn cache_key_does_not_panic_on_a_short_hash() {
        let fp = Fingerprint {
            size: 10,
            mtime_unix: 0,
            content_hash: "ab".to_string(),
        };
        assert_eq!(fp.cache_key(), "ab-10");
    }
```

Unit tests sit in a `#[cfg(test)] mod tests` block in the same file, and may use the
private items around them. Property tests generate inputs:

```rust
// crates/lumit-core/src/time.rs — `add_sub_round_trip`
    proptest! {
        #[test]
        fn add_sub_round_trip(a in -1_000_000i64..1_000_000, b in 1i64..100_000,
                              c in -1_000_000i64..1_000_000, d in 1i64..100_000) {
            let x = rat(a, b);
            let y = rat(c, d);
            let back = x.checked_add(y).unwrap().checked_sub(y).unwrap();
            prop_assert_eq!(back, x);
        }
```

Test names in this repo are sentences describing the guarantee. Follow that.

Pure functions get table tests, which is why the scheduler is deliberately pure:

```rust
// crates/lumit-eval/src/schedule.rs — `next_frame_to_schedule`
pub fn next_frame_to_schedule<T>(
    clock_frame: u64,
    target_frame: u64,
    ring: &FrameRing<T>,
    already_scheduled: impl Fn(u64) -> bool,
) -> Option<u64> {
    if target_frame < clock_frame {
        return None;
    }
    (clock_frame..=target_frame).find(|&n| !ring.contains(n) && !already_scheduled(n))
}
```

## Reading order in this repo

1. `crates/lumit-core/src/time.rs` — small, self-contained, heavily commented.
2. `crates/lumit-core/src/anim.rs` — enums, matching, real maths.
3. `crates/lumit-core/src/ops.rs` — the command pattern. Skim it, do not read it whole.
4. `crates/lumit-eval/src/schedule.rs` — pure logic with excellent tests.
5. `crates/lumit-bridge/src/api/keymap.rs` — a small, complete bridge module.

## The house rules that will surprise you

- The workspace denies `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!` and
  `unsafe`. Tests re-allow them locally.
- Never hold a lock across an await, a GPU call, or a render.
- Every `Op` returns its exact inverse. That is how undo works.
- Engine crates never depend on the bridge or any UI.
- A failure degrades to a picture, never an error the user sees as a crash.
