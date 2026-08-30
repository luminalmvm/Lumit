//! Registration: the list of effects this build has (docs/impl/
//! effect-registry.md §2.6).
//!
//! **In plain terms.** This is the whole of "adding an effect to Lumit": one
//! line, naming the thing that carries the effect's behaviour and the block that
//! declares its controls. The order here is the order the Add-effect menu, the
//! command palette and the preset browser show (K-137), so it is deliberately a
//! written list rather than something assembled at start-up in whatever order
//! the linker happened to choose.
//!
//! Effects that are not known when Lumit is compiled — OFX plugins (docs/12),
//! and in time the user's own — arrive through the same [`EffectDef`](crate::fx::
//! EffectDef) trait object at run time. That is the seam this arrangement exists
//! for; nothing here is a closed set any more.
//!
//! Both halves of the catalogue come from this one list: `BUILTIN_DEFS`, the
//! behaviours the frame walk dispatches through, and `BUILTINS`, the
//! declarations the menu and the bridge read. Each line's left side is the
//! effect's behaviour, its right side the parameter block `#[derive(Effect)]`
//! generated the declaration from.

use super::drivers::{
    audio_level::{AudioLevel, AudioLevelDef},
    colour_cycle::{ColourCycle, ColourCycleDef},
    layer_points::{LayerPoints, LayerPointsDef},
    math::{Math, MathDef},
    points_sample::{PointsSample, PointsSampleDef},
    remap::{Remap, RemapDef},
    smooth::{Smooth, SmoothDef},
    wiggle::{Wiggle, WiggleDef},
};
use super::effects::{
    accumulation_mb::{AccumulationMb, AccumulationMbDef},
    add_grain::{AddGrain, AddGrainDef},
    angle_control::{AngleControl, AngleControlDef},
    beam::{Beam, BeamDef},
    bezier_warp::{BezierWarp, BezierWarpDef},
    black_and_white::{BlackAndWhite, BlackAndWhiteDef},
    block_glitch::{BlockGlitch, BlockGlitchDef},
    blur::{Blur, BlurDef},
    brightness::{Brightness, BrightnessDef},
    broadcast_safe::{BroadcastSafe, BroadcastSafeDef},
    camera_track::{CameraTrack, CameraTrackDef},
    card_wipe::{CardWipe, CardWipeDef},
    channel_blur::{ChannelBlur, ChannelBlurDef},
    checkbox_control::{CheckboxControl, CheckboxControlDef},
    chromatic_aberration::{ChromaticAberration, ChromaticAberrationDef},
    clone_to_points::{CloneToPoints, CloneToPointsDef},
    colour_balance::{ColourBalance, ColourBalanceDef},
    colour_control::{ColourControl, ColourControlDef},
    connect_points::{ConnectPoints, ConnectPointsDef},
    contrast::{Contrast, ContrastDef},
    corner_pin::{CornerPin, CornerPinDef},
    curves::{Curves, CurvesDef},
    datamosh::{Datamosh, DatamoshDef},
    directional_blur::{DirectionalBlur, DirectionalBlurDef},
    displacement_map::{DisplacementMap, DisplacementMapDef},
    dof::{Dof, DofDef},
    drop_shadow::{DropShadow, DropShadowDef},
    echo::{Echo, EchoDef},
    emboss::{Emboss, EmbossDef},
    emit_from_image::{EmitFromImage, EmitFromImageDef},
    exposure::{Exposure, ExposureDef},
    fill::{Fill, FillDef},
    find_edges::{FindEdges, FindEdgesDef},
    flash::{Flash, FlashDef},
    fractal_noise::{FractalNoise, FractalNoiseDef},
    gamma::{Gamma, GammaDef},
    glow::{Glow, GlowDef},
    gradient::{Gradient, GradientDef},
    grid::{Grid, GridDef},
    hue_saturation::{HueSaturation, HueSaturationDef},
    hue_shift::{HueShift, HueShiftDef},
    invert::{Invert, InvertDef},
    iris_wipe::{IrisWipe, IrisWipeDef},
    lens_distort::{LensDistort, LensDistortDef},
    lens_flare::{LensFlare, LensFlareDef},
    levels::{Levels, LevelsDef},
    light_wrap::{LightWrap, LightWrapDef},
    lightning::{Lightning, LightningDef},
    linear_wipe::{LinearWipe, LinearWipeDef},
    lut::{Lut, LutDef},
    matte_key::{MatteKey, MatteKeyDef},
    median::{Median, MedianDef},
    mirror::{Mirror, MirrorDef},
    mosaic::{Mosaic, MosaicDef},
    motion_blur::{MotionBlur, MotionBlurDef},
    noise::{Noise, NoiseDef},
    offset::{Offset, OffsetDef},
    particulate::{Particulate, ParticulateDef},
    photo_filter::{PhotoFilter, PhotoFilterDef},
    planar_track::{PlanarTrack, PlanarTrackDef},
    point_control::{PointControl, PointControlDef},
    polar_coordinates::{PolarCoordinates, PolarCoordinatesDef},
    posterize::{Posterize, PosterizeDef},
    posterize_time::{PosterizeTime, PosterizeTimeDef},
    radial_blur::{RadialBlur, RadialBlurDef},
    radial_wipe::{RadialWipe, RadialWipeDef},
    radio_waves::{RadioWaves, RadioWavesDef},
    rgb_split::{RgbSplit, RgbSplitDef},
    ripple::{Ripple, RippleDef},
    roughen_edges::{RoughenEdges, RoughenEdgesDef},
    saturation::{Saturation, SaturationDef},
    scanlines::{Scanlines, ScanlinesDef},
    scatter::{Scatter, ScatterDef},
    scribble::{Scribble, ScribbleDef},
    set_channels::{SetChannels, SetChannelsDef},
    set_matte::{SetMatte, SetMatteDef},
    shadow_highlight::{ShadowHighlight, ShadowHighlightDef},
    shake::{Shake, ShakeDef},
    sharpen::{Sharpen, SharpenDef},
    sharpen_simple::{SharpenSimple, SharpenSimpleDef},
    slider_control::{SliderControl, SliderControlDef},
    spherize::{Spherize, SpherizeDef},
    sprite_flare::{SpriteFlare, SpriteFlareDef},
    stroke::{Stroke, StrokeDef},
    temperature::{Temperature, TemperatureDef},
    texturize::{Texturize, TexturizeDef},
    threshold::{Threshold, ThresholdDef},
    tile::{Tile, TileDef},
    tint::{Tint, TintDef},
    trail::{Trail, TrailDef},
    transform::{Transform, TransformDef},
    tritone::{Tritone, TritoneDef},
    turbulent_displace::{TurbulentDisplace, TurbulentDisplaceDef},
    twirl::{Twirl, TwirlDef},
    vegas::{Vegas, VegasDef},
    venetian_blinds::{VenetianBlinds, VenetianBlindsDef},
    vibrancy::{Vibrancy, VibrancyDef},
    vignette::{Vignette, VignetteDef},
    warp::{Warp, WarpDef},
    wave_warp::{WaveWarp, WaveWarpDef},
};

crate::catalogue![
    BlurDef => Blur,
    DirectionalBlurDef => DirectionalBlur,
    RadialBlurDef => RadialBlur,
    SharpenDef => Sharpen,
    SharpenSimpleDef => SharpenSimple,
    SpriteFlareDef => SpriteFlare,
    LightWrapDef => LightWrap,
    RgbSplitDef => RgbSplit,
    ChromaticAberrationDef => ChromaticAberration,
    FlashDef => Flash,
    ColourBalanceDef => ColourBalance,
    SaturationDef => Saturation,
    VibrancyDef => Vibrancy,
    VignetteDef => Vignette,
    ExposureDef => Exposure,
    HueShiftDef => HueShift,
    ContrastDef => Contrast,
    GammaDef => Gamma,
    TemperatureDef => Temperature,
    LutDef => Lut,
    DofDef => Dof,
    // The utility batch's blur (K-400), at the Blur & sharpen family's end.
    ChannelBlurDef => ChannelBlur,
    TransformDef => Transform,
    GlowDef => Glow,
    ShakeDef => Shake,
    BlockGlitchDef => BlockGlitch,
    ScanlinesDef => Scanlines,
    DatamoshDef => Datamosh,
    // The distort batch, appended at the Distortion family's end (K-137), in
    // docs/08 §3.38–§3.42 order.
    TurbulentDisplaceDef => TurbulentDisplace,
    TileDef => Tile,
    OffsetDef => Offset,
    MirrorDef => Mirror,
    LensDistortDef => LensDistort,
    // Wave 2's Distort I batch, appended at the Distortion family's end
    // (K-137), in docs/08 §3.48-§3.52 order.
    CornerPinDef => CornerPin,
    DisplacementMapDef => DisplacementMap,
    PolarCoordinatesDef => PolarCoordinates,
    TwirlDef => Twirl,
    SpherizeDef => Spherize,
    // Wave 2's Distort II batch, appended at the Distortion family's end
    // (K-137), in docs/08 §3.53-§3.56 order.
    RippleDef => Ripple,
    WaveWarpDef => WaveWarp,
    BezierWarpDef => BezierWarp,
    WarpDef => Warp,
    // The Generate family (K-398), in docs/08 §3.34–§3.37 order.
    FillDef => Fill,
    GradientDef => Gradient,
    NoiseDef => Noise,
    FractalNoiseDef => FractalNoise,
    // Wave 2's Draw and grain batch (K-407), appended at the Generate family's
    // end (K-137), in docs/08 §3.73-§3.77 order. All five draw or lay something
    // over the frame rather than change the colour of what is there, which is
    // what K-398 opened this category for.
    BeamDef => Beam,
    LightningDef => Lightning,
    RadioWavesDef => RadioWaves,
    VegasDef => Vegas,
    AddGrainDef => AddGrain,
    // K-408's consumers, after them and in docs/08 §3.78-§3.79 order: the two
    // effects that draw a mask's own line rather than one found in the picture.
    ScribbleDef => Scribble,
    StrokeDef => Stroke,
    // Particulate (K-446, K-491), at the Generate family's end: it makes its
    // own pixels rather than changing the ones that arrived, which is what
    // K-398 opened this category for, and it is the first entry to declare a
    // data output beside its picture (K-472).
    ParticulateDef => Particulate,
    // The generators (K-598, K-599, K-603), beside it and for the same reason:
    // they make points rather than pixels, and declare the same Points output.
    GridDef => Grid,
    ScatterDef => Scatter,
    EmitFromImageDef => EmitFromImage,
    // The consumers (K-600, K-601, K-602), after the producers they read: the
    // stack effects that take a points wire rather than hand one out.
    CloneToPointsDef => CloneToPoints,
    TrailDef => Trail,
    ConnectPointsDef => ConnectPoints,
    EchoDef => Echo,
    PosterizeTimeDef => PosterizeTime,
    AccumulationMbDef => AccumulationMb,
    MotionBlurDef => MotionBlur,
    MatteKeyDef => MatteKey,
    // Set matte, at the Utility family's end (K-400).
    SetMatteDef => SetMatte,
    // Set channels, beside it: the same question asked of all four channels
    // rather than only of the alpha.
    SetChannelsDef => SetChannels,
    // Broadcast safe (docs/08 §3.69, K-405), after it — a delivery tool rather
    // than a look, which is what Utility is for.
    BroadcastSafeDef => BroadcastSafe,
    // Camera track (docs/08 §3.85, K-417), at the Utility family's end. It is a
    // handle for a background analysis rather than an image operation, which is
    // exactly the kind of thing Utility is for.
    CameraTrackDef => CameraTrack,
    // Planar track (docs/08 §3.87, K-579), beside it: the same substrate asked
    // a different question — where one flat surface is, rather than where the
    // camera was — and a handle for a background analysis in exactly the same
    // way, which is what puts it here rather than in Distortion beside the
    // Corner pin it writes.
    PlanarTrackDef => PlanarTrack,
    InvertDef => Invert,
    TintDef => Tint,
    CurvesDef => Curves,
    LevelsDef => Levels,
    BrightnessDef => Brightness,
    HueSaturationDef => HueSaturation,
    // Wave 2's Stylise I batch (K-404), appended at the Colour family's end
    // (K-137), in docs/08 §3.58-§3.63 order. AE files all six under Color
    // Correction and so does Lumit: every one of them is tone or colour maths.
    PosterizeDef => Posterize,
    ThresholdDef => Threshold,
    TritoneDef => Tritone,
    PhotoFilterDef => PhotoFilter,
    BlackAndWhiteDef => BlackAndWhite,
    ShadowHighlightDef => ShadowHighlight,
    LensFlareDef => LensFlare,
    // Drop shadow, at the Stylise family's end (K-400).
    DropShadowDef => DropShadow,
    // Roughen edges (docs/08 §3.57), at the Stylise family's end after it.
    RoughenEdgesDef => RoughenEdges,
    // Wave 2's Stylise II batch (K-405), appended at the Stylise family's end
    // (K-137), in docs/08 §3.64-§3.68 order. These five really are stylisations
    // — what each does to a frame is change how it *looks*, not what colour a
    // pixel is — which is what puts them here and Stylise I in Colour.
    MedianDef => Median,
    MosaicDef => Mosaic,
    FindEdgesDef => FindEdges,
    EmbossDef => Emboss,
    TexturizeDef => Texturize,
    // The Transition family (K-400), in docs/08 §3.46–§3.47 order.
    LinearWipeDef => LinearWipe,
    RadialWipeDef => RadialWipe,
    // Wave 2's Transitions batch (K-406), appended at the Transition family's
    // end (K-137), in docs/08 §3.70-§3.72 order.
    VenetianBlindsDef => VenetianBlinds,
    IrisWipeDef => IrisWipe,
    CardWipeDef => CardWipe,
    // The Controls family (K-414), last in the catalogue and so last in the
    // Add-effect menu, which groups by first appearance here (K-137). The order
    // inside it is After Effects' own Expression Controls order, which is what
    // a hand arriving from AE will look for.
    SliderControlDef => SliderControl,
    AngleControlDef => AngleControl,
    CheckboxControlDef => CheckboxControl,
    ColourControlDef => ColourControl,
    PointControlDef => PointControl,
    // The Drivers family (K-471), last of all: a driver is added from the Graph
    // panel's own search rather than from the Add-effect menu, and its entries
    // declare a data signature instead of an image kernel. The order inside it
    // is the order docs/impl/node-graph.md §1.3 lists them.
    WiggleDef => Wiggle,
    AudioLevelDef => AudioLevel,
    ColourCycleDef => ColourCycle,
    MathDef => Math,
    RemapDef => Remap,
    SmoothDef => Smooth,
    // The first driver that reads data rather than only numbers (K-492,
    // K-494): a Points stream in, a count and a distance out. Last, because it
    // arrived last and the order here is the order the Graph panel's search
    // shows.
    PointsSampleDef => PointsSample,
    // The family's cross-layer tap (K-604): the first driver whose *output* is
    // a stream rather than a number, and the one node that reaches outside the
    // layer its graph belongs to.
    LayerPointsDef => LayerPoints,
];
