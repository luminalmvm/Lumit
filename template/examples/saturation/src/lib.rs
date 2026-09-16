//! An LFX plugin in Rust, through the `lfx` wrapper.
//!
//! # In plain terms
//!
//! It pulls the picture towards its own luminance, or past it. The effect is
//! four lines; what the file is here to show is how little else there is.
//!
//! One trait and one macro is the whole of the plumbing. The entry table, the
//! instance lifetime, the strided value array, both pixel depths and the
//! panic net are the wrapper's; the maths, the declarations and the trait
//! block are the author's, and that division is deliberate - each of those
//! three is a decision nobody else can make for an effect.
//!
//! Two of them are worth reading twice. `Traits::per_pixel` says this effect
//! reads the output pixel and nothing around it, which is what lets the host
//! tile the frame - a kernel that reached further while saying it would
//! produce seams. And the version in `SPEC` re-keys every cached frame, so a
//! release whose maths moved must move one of its three numbers, or the host
//! will go on serving the pictures the old maths made and be right to.
//!
//! # Thread role and contract
//!
//! `describe` runs on the host's control thread; `process` runs on any worker
//! thread, and on several instances of this effect at once. One instance is
//! never re-entered, which is why nothing here locks anything.

/// The order the controls are declared in is the order they are drawn and the
/// order the value array arrives in.
const AMOUNT: u32 = 0;
const PRESERVE_LUMA: u32 = 1;

/// Rec. 709 luminance, which is the weighting the working space is written in.
const LUMA: [f32; 3] = [0.212_639, 0.715_169, 0.072_192];

/// One live effect. There is nothing in it, and there is nothing in it for a
/// reason: everything this effect needs arrives with every frame, so there is
/// nothing the host could fail to hash into the frame key.
#[derive(Default)]
struct Saturation;

impl lfx::Effect for Saturation {
    const SPEC: lfx::Spec = lfx::Spec {
        id: c"com.example.lfx.saturation",
        name: c"Saturation",
        vendor: c"Example",
        version: (1, 0, 0),
        // The first is the heading the effect is browsed under; the rest are
        // search keywords.
        categories: &[lfx::Category::Colour, lfx::Category::Utility],
        traits: lfx::Traits::per_pixel(lfx::Cost::Cheap),
        // Nothing beyond the frozen core, so nothing to negotiate - which is
        // the list that always loads. Version 1 of the host offers no
        // extension table at all.
        required_extensions: &[],
    };

    fn describe(&mut self, sink: &mut lfx::Describe<'_>) -> bool {
        // A share of something, so per cent; 100 is the picture as it arrived,
        // nought is greyscale, and past 100 is where the control earns its
        // range.
        sink.slider(
            c"amount",
            c"Amount",
            lfx::Unit::Percent,
            100.0,
            -100.0,
            300.0,
        );
        sink.flag(c"preserve_luma", c"Preserve luminance", true);
        true
    }

    fn process(&mut self, call: &lfx::Request<'_>) -> lfx::Status {
        let values = call.values();
        let amount = values.number(AMOUNT, 100.0) as f32 * 0.01;
        let preserve = values.flag(PRESERVE_LUMA, true);

        call.for_each_pixel(|_, _, [r, g, b, a]| {
            let luma = LUMA[0] * r + LUMA[1] * g + LUMA[2] * b;
            let mixed = [
                luma + (r - luma) * amount,
                luma + (g - luma) * amount,
                luma + (b - luma) * amount,
            ];
            if !preserve {
                return [mixed[0], mixed[1], mixed[2], a];
            }
            // Pulling the channels apart moves the luminance unless it is put
            // back, which is what "preserve luminance" means and what a
            // saturation control usually wants.
            let moved = LUMA[0] * mixed[0] + LUMA[1] * mixed[1] + LUMA[2] * mixed[2];
            let lift = luma - moved;
            [mixed[0] + lift, mixed[1] + lift, mixed[2] + lift, a]
        })
    }
}

lfx::bundle!(Saturation);
