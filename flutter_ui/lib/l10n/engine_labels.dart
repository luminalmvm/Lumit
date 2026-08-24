/// In plain terms: some of the words on screen are the engine's, not this
/// application's. The effects and their controls name themselves —
/// "Gaussian blur", "Radius", "Blur & sharpen"
/// (`crates/lumit-core/src/fx/effects/`, listed in `fx-labels.txt`) — and so does every keyboard
/// shortcut in Settings ▸ Keymap: "Play or pause", "Anywhere"
/// (`crates/lumit-keymap/src/lib.rs`). All of it arrives over the bridge as
/// plain English. It is the only user-facing text Lumit shows that is not
/// written in Dart, and without this table the Effect controls panel and the
/// Keymap page would read as English islands inside a translated application.
///
/// Rather than teach the engine about languages, this table looks each name up
/// by the English text the engine already sent. Nothing in Rust changes, no
/// second set of identifiers has to be kept in step with the schema, and a name
/// nobody has translated yet simply comes back as it arrived.
///
/// Two known limits, both small and both deliberate:
///
/// * The lookup is by *word*, not by place. Two controls that are both called
///   "Scale" in English get the same translation even if the language would
///   rather they did not. The fix, when it bites, is to give the affected label
///   distinct English text in the schema — good practice anyway.
/// * The two shortcut labels the engine builds with a number in them ("Add
///   marker 3 at the playhead", "Go to marker 3") are not literals in Rust and
///   so are not in this table. They stay English until the engine hands their
///   number over separately.
///
/// `test/l10n/engine_labels_test.dart` reads both Rust sources and fails if
/// either can send a word this table has no entry for, so an effect or a
/// shortcut added to the engine cannot quietly ship untranslated.
library;

import 'package:lumit_flutter/l10n/strings.dart';

/// The translation of an engine-supplied label, or [english] unchanged if there
/// is no entry for it.
String engineLabel(String english) => _table[english] ?? english;

/// Whether [english] is a label this table knows — what the sync test asserts.
bool hasEngineLabel(String english) => _table.containsKey(english);

Map<String, String> get _table => {
      "Add grain": l10n.fxAddGrain,
      "Add": l10n.fxAdd,
      "All edges": l10n.fxAllEdges,
      "Alpha": l10n.fxAlpha,
      "Alpha bias": l10n.fxAlphaBias,
      "Alpha blur": l10n.fxAlphaBlur,
      "Amount": l10n.fxAmount,
      "Amplitude": l10n.fxAmplitude,
      "Analyse": l10n.fxAnalyse,
      "Anamorphic squeeze": l10n.fxAnamorphicSqueeze,
      "Anchor x": l10n.fxAnchorX,
      "Anchor y": l10n.fxAnchorY,
      "Angle": l10n.fxAngle,
      "Angle control": l10n.fxAngleControl,
      "Animate": l10n.fxAnimate,
      "Anticlockwise": l10n.fxAnticlockwise,
      "Aperture": l10n.fxAperture,
      "Arc": l10n.fxArc,
      "Arc lower": l10n.fxArcLower,
      "Arc upper": l10n.fxArcUpper,
      "Arch": l10n.fxArch,
      "Aspect ratio": l10n.fxAspectRatio,
      "Background": l10n.fxBackground,
      "Backwards": l10n.fxBackwards,
      "Beam": l10n.fxBeam,
      "Behind": l10n.fxBehind,
      "Blades": l10n.fxBlades,
      "Blend": l10n.fxBlend,
      "Block glitch": l10n.fxBlockGlitch,
      "Block size": l10n.fxBlockSize,
      "Bloom": l10n.fxBloom,
      "Blue": l10n.fxBlue,
      "Blur & sharpen": l10n.fxBlurAndSharpen,
      "Bottom to top": l10n.fxBottomToTop,
      "Brush hardness": l10n.fxBrushHardness,
      "Brush size": l10n.fxBrushSize,
      "Camera track": l10n.fxCameraTrack,
      "Cancel": l10n.fxCancel,
      "Card wipe": l10n.fxCardWipe,
      "Centre X": l10n.fxCentreX,
      "Centre Y": l10n.fxCentreY,
      "Channel offset": l10n.fxChannelOffset,
      "Checkbox": l10n.fxCheckbox,
      "Checkbox control": l10n.fxCheckboxControl,
      "Chromatic aberration": l10n.fxChromaticAberration,
      "Clip black": l10n.fxClipBlack,
      "Clip rollback": l10n.fxClipRollback,
      "Clip white": l10n.fxClipWhite,
      "Coating": l10n.fxCoating,
      "Colour burn": l10n.blendColourBurn,
      "Colour control": l10n.fxColourControl,
      "Colour dodge": l10n.blendColourDodge,
      "Composite on original": l10n.fxCompositeOnOriginal,
      "Conductivity state": l10n.fxConductivityState,
      "Controls": l10n.fxControls,
      "Core colour": l10n.fxCoreColour,
      "Core radius": l10n.fxCoreRadius,
      "Darker colour": l10n.blendDarkerColour,
      "Columns": l10n.fxColumns,
      "Direction x": l10n.fxDirectionX,
      "Direction y": l10n.fxDirectionY,
      "Element 1": l10n.fxElement1,
      "Element 2": l10n.fxElement2,
      "Element 3": l10n.fxElement3,
      "Element 4": l10n.fxElement4,
      "Element 5": l10n.fxElement5,
      "Element 6": l10n.fxElement6,
      "Element 7": l10n.fxElement7,
      "Element 8": l10n.fxElement8,
      "Element 9": l10n.fxElement9,
      "Element 10": l10n.fxElement10,
      "Element 11": l10n.fxElement11,
      "Element 12": l10n.fxElement12,
      "Element 13": l10n.fxElement13,
      "Element 14": l10n.fxElement14,
      "Element 15": l10n.fxElement15,
      "Element 16": l10n.fxElement16,
      "Element 17": l10n.fxElement17,
      "Element 18": l10n.fxElement18,
      "Element 19": l10n.fxElement19,
      "Element 20": l10n.fxElement20,
      "As the lens file": l10n.fxCoatingAsFile,
      "Asymmetric": l10n.fxAsymmetric,
      "Basic": l10n.fxBasic,
      "Bend": l10n.fxBend,
      "Bezier warp": l10n.fxBezierWarp,
      "Black and white": l10n.fxBlackAndWhite,
      "Blue blur": l10n.fxBlueBlur,
      "Blues": l10n.fxBlues,
      "Border": l10n.fxBorder,
      "Both": l10n.fxBoth,
      "Bottom edge": l10n.fxBottomEdge,
      "Bottom left tangent x": l10n.fxBottomLeftTangentX,
      "Bottom left tangent y": l10n.fxBottomLeftTangentY,
      "Bottom right tangent x": l10n.fxBottomRightTangentX,
      "Bottom right tangent y": l10n.fxBottomRightTangentY,
      "Brightness": l10n.fxBrightness,
      "End": l10n.fxEnd,
      "End thickness": l10n.fxEndThickness,
      "Expansion": l10n.fxExpansion,
      "Fade in": l10n.fxFadeIn,
      "Fade out": l10n.fxFadeOut,
      "Forking": l10n.fxForking,
      "Glow colour": l10n.fxGlowColour,
      "Glow opacity": l10n.fxGlowOpacity,
      "Glow radius": l10n.fxGlowRadius,
      "Hard mix": l10n.blendHardMix,
      "Hardness": l10n.fxHardness,
      "Hue": l10n.blendHue,
      "Inside colour": l10n.fxInsideColour,
      "Inside mask": l10n.fxInsideMask,
      "Jagged": l10n.fxJagged,
      "Lifespan": l10n.fxLifespan,
      "Lighter colour": l10n.blendLighterColour,
      "Lightning": l10n.fxLightning,
      "Linear burn": l10n.blendLinearBurn,
      "Linear light": l10n.blendLinearLight,
      "Luminosity": l10n.blendLuminosity,
      "Mask": l10n.fxMask,
      "Mask/Path": l10n.fxMaskPath,
      "Monochrome": l10n.fxMonochrome,
      "Omni": l10n.fxOmni,
      "On original": l10n.fxOnOriginal,
      "On transparent": l10n.fxOnTransparent,
      "Origin x": l10n.fxOriginX,
      "Origin y": l10n.fxOriginY,
      "Outside colour": l10n.fxOutsideColour,
      "Outside mask": l10n.fxOutsideMask,
      "Paint style": l10n.fxPaintStyle,
      "Path overlap": l10n.fxPathOverlap,
      "Pin light": l10n.blendPinLight,
      "Flip axis": l10n.fxFlipAxis,
      "Flip direction": l10n.fxFlipDirection,
      "Flip order": l10n.fxFlipOrder,
      "Forwards": l10n.fxForwards,
      "Horizontal axis": l10n.fxHorizontalAxis,
      "Inner radius": l10n.fxInnerRadius,
      "Iris centre x": l10n.fxIrisCentreX,
      "Iris centre y": l10n.fxIrisCentreY,
      "Iris points": l10n.fxIrisPoints,
      "Iris wipe": l10n.fxIrisWipe,
      "Left to right": l10n.fxLeftToRight,
      "Outer radius": l10n.fxOuterRadius,
      "Point control": l10n.fxPointControl,
      "Point x": l10n.fxPointX,
      "Point y": l10n.fxPointY,
      "Producer x": l10n.fxProducerX,
      "Producer y": l10n.fxProducerY,
      "Radio waves": l10n.fxRadioWaves,
      "Random": l10n.fxRandom,
      "Randomness": l10n.fxRandomness,
      "Reveal original": l10n.fxRevealOriginal,
      "Right to left": l10n.fxRightToLeft,
      "Rows": l10n.fxRows,
      "Scribble": l10n.fxScribble,
      "Segment length": l10n.fxSegmentLength,
      "Sides": l10n.fxSides,
      "Slider": l10n.fxSlider,
      "Slider control": l10n.fxSliderControl,
      "Spacing": l10n.fxSpacing,
      "Star depth": l10n.fxStarDepth,
      "sRGB": l10n.fxSrgb,
      "Star": l10n.fxStar,
      "Start": l10n.fxStart,
      "Start thickness": l10n.fxStartThickness,
      "Static": l10n.fxStatic,
      "Strike": l10n.fxStrike,
      "Stroke": l10n.fxStroke,
      "Stroke width": l10n.fxStrokeWidth,
      "Time": l10n.fxTime,
      "Top to bottom": l10n.fxTopToBottom,
      "Transition width": l10n.fxTransitionWidth,
      "Two-way strike": l10n.fxTwoWayStrike,
      "Uncoated": l10n.fxCoatingUncoated,
      "Single layer, straw": l10n.fxCoatingSingleStraw,
      "Two layer, magenta": l10n.fxCoatingTwoMagenta,
      "Broadband, green": l10n.fxCoatingBroadGreen,
      "Broadband, amber": l10n.fxCoatingBroadAmber,
      "Broadband, blue": l10n.fxCoatingBroadBlue,
      "Broadcast safe": l10n.fxBroadcastSafe,
      "Bulge": l10n.fxBulge,
      "Centre": l10n.fxCentre,
      "Channel": l10n.fxChannel,
      "Channel blur": l10n.fxChannelBlur,
      "Circle": l10n.fxCircle,
      "Clockwise": l10n.fxClockwise,
      "Colour": l10n.fxColour,
      "Colour 1": l10n.fxColour1,
      "Colour 2": l10n.fxColour2,
      "Colour 3": l10n.fxColour3,
      "Colour balance": l10n.fxColourBalance,
      "Colour correction": l10n.fxColourCorrection,
      "Colour edge": l10n.fxColourEdge,
      "Colour noise": l10n.fxColourNoise,
      "Combine with existing alpha": l10n.fxCombineWithExistingAlpha,
      "Completion": l10n.fxCompletion,
      "Complexity": l10n.fxComplexity,
      "Confidence": l10n.fxConfidence,
      "Contrast": l10n.fxContrast,
      "Conversion": l10n.fxConversion,
      "Cooling filter (80)": l10n.fxCoolingFilter80,
      "Cooling filter (82)": l10n.fxCoolingFilter82,
      "Cooling filter (LBB)": l10n.fxCoolingFilterLbb,
      "Corner pin": l10n.fxCornerPin,
      "Curves": l10n.fxCurves,
      "Custom": l10n.fxCustom,
      "Cut": l10n.fxCut,
      "Cyan": l10n.fxCyan,
      "Cyans": l10n.fxCyans,
      "Cycle": l10n.fxCycle,
      "Cycle evolution": l10n.fxCycleEvolution,
      "Darken": l10n.fxDarken,
      "Datamosh": l10n.fxDatamosh,
      "Decay": l10n.fxDecay,
      "Deep blue": l10n.fxDeepBlue,
      "Deep emerald": l10n.fxDeepEmerald,
      "Deep red": l10n.fxDeepRed,
      "Deep yellow": l10n.fxDeepYellow,
      "Density": l10n.fxDensity,
      "Depth channel": l10n.fxDepthChannel,
      "Depth invert": l10n.fxDepthInvert,
      "Depth layer": l10n.fxDepthLayer,
      "Depth map": l10n.fxDepthMap,
      "Depth of field": l10n.fxDepthOfField,
      "Despill amount": l10n.fxDespillAmount,
      "Despill bias": l10n.fxDespillBias,
      "Despot black": l10n.fxDespotBlack,
      "Despot white": l10n.fxDespotWhite,
      "Detail": l10n.fxDetail,
      "Detect edge threshold": l10n.fxDetectEdgeThreshold,
      "Diagonal": l10n.fxDiagonal,
      "Difference": l10n.fxDifference,
      "Direction": l10n.fxDirection,
      "Directional blur": l10n.fxDirectionalBlur,
      "Dispersion": l10n.fxDispersion,
      "Displacement": l10n.fxDisplacement,
      "Displacement map": l10n.fxDisplacementMap,
      "Display": l10n.fxDisplay,
      "Distance": l10n.fxDistance,
      "Distortion": l10n.fxDistortion,
      "Distribution": l10n.fxDistribution,
      "Divide": l10n.fxDivide,
      "Dominant motion": l10n.fxDominantMotion,
      "Draft": l10n.fxDraft,
      "Drop shadow": l10n.fxDropShadow,
      "Duration": l10n.fxDuration,
      "Echo": l10n.fxEcho,
      "Echoes": l10n.fxEchoes,
      "Edge colour": l10n.fxEdgeColour,
      "Edge sharpness": l10n.fxEdgeSharpness,
      "Edge type": l10n.fxEdgeType,
      "Edges": l10n.fxEdges,
      "Emboss": l10n.fxEmboss,
      "End colour": l10n.fxEndColour,
      "End x": l10n.fxEndX,
      "End y": l10n.fxEndY,
      "Every Nth beat": l10n.fxEveryNthBeat,
      "Evolution": l10n.fxEvolution,
      "Evolution options": l10n.fxEvolutionOptions,
      "Exclusion": l10n.fxExclusion,
      "Exposure": l10n.fxExposure,
      "F-stop": l10n.fxFStop,
      "Fade": l10n.fxFade,
      "Far blur": l10n.fxFarBlur,
      "Fast motion blur": l10n.fxFastMotionBlur,
      "Feature density": l10n.fxFeatureDensity,
      "Feather": l10n.fxFeather,
      "Field of view": l10n.fxFieldOfView,
      "File": l10n.fxFile,
      "Fill": l10n.fxFill,
      "Filter": l10n.fxFilter,
      "Final result": l10n.fxFinalResult,
      "Find edges": l10n.fxFindEdges,
      "Fish": l10n.fxFish,
      "Fisheye": l10n.fxFisheye,
      "Flag": l10n.fxFlag,
      "Flare options": l10n.fxFlareOptions,
      "Flash": l10n.fxFlash,
      "Focus (m)": l10n.fxFocusM,
      "Focus distance": l10n.fxFocusDistance,
      "Focus map": l10n.fxFocusMap,
      "Focus point x": l10n.fxFocusPointX,
      "Focus point y": l10n.fxFocusPointY,
      "Focus range": l10n.fxFocusRange,
      "Force on all layers": l10n.fxForceOnAllLayers,
      "Fractal influence": l10n.fxFractalInfluence,
      "Fractal noise": l10n.fxFractalNoise,
      "Fractal type": l10n.fxFractalType,
      "Frame rate": l10n.fxFrameRate,
      "Frequency": l10n.fxFrequency,
      "Gain": l10n.fxGain,
      "Gamma": l10n.fxGamma,
      "Gaussian": l10n.fxGaussian,
      "Gaussian blur": l10n.fxGaussianBlur,
      "Generate": l10n.fxGenerate,
      "Ghost intensity": l10n.fxGhostIntensity,
      "Ghost size": l10n.fxGhostSize,
      "Ghost softness": l10n.fxGhostSoftness,
      "Ghost spacing": l10n.fxGhostSpacing,
      "Ghosts": l10n.fxGhosts,
      "Glow": l10n.fxGlow,
      "Glow intensity": l10n.fxGlowIntensity,
      "Glow size": l10n.fxGlowSize,
      "Gradient": l10n.fxGradient,
      "Green": l10n.fxGreen,
      "Green blur": l10n.fxGreenBlur,
      "Greens": l10n.fxGreens,
      "Hard": l10n.fxHard,
      "Hard colour": l10n.fxHardColour,
      "Hard light": l10n.fxHardLight,
      "High": l10n.fxHigh,
      "Highlight amount": l10n.fxHighlightAmount,
      "Highlight exposure": l10n.fxHighlightExposure,
      "Highlight threshold": l10n.fxHighlightThreshold,
      "Highlight tonal width": l10n.fxHighlightTonalWidth,
      "Highlights": l10n.fxHighlights,
      "Horizontal": l10n.fxHorizontal,
      "Horizontal amount": l10n.fxHorizontalAmount,
      "Horizontal blocks": l10n.fxHorizontalBlocks,
      "Horizontal channel": l10n.fxHorizontalChannel,
      "Horizontal distortion": l10n.fxHorizontalDistortion,
      "Horizontal phase shift": l10n.fxHorizontalPhaseShift,
      "How to treat": l10n.fxHowToTreat,
      "Hue and saturation": l10n.fxHueAndSaturation,
      "Hue shift": l10n.fxHueShift,
      "In front": l10n.fxInFront,
      "Inflate": l10n.fxInflate,
      "Input black": l10n.fxInputBlack,
      "Input space": l10n.fxInputSpace,
      "Input white": l10n.fxInputWhite,
      "Intensity": l10n.fxIntensity,
      "Interlace offset": l10n.fxInterlaceOffset,
      "Interpolation": l10n.fxInterpolation,
      "Invert": l10n.fxInvert,
      "Invert edges": l10n.fxInvertEdges,
      "Invert noise": l10n.fxInvertNoise,
      "Iris": l10n.fxIris,
      "Key out safe": l10n.fxKeyOutSafe,
      "Key out unsafe": l10n.fxKeyOutUnsafe,
      "Left and right": l10n.fxLeftAndRight,
      "Left bottom tangent x": l10n.fxLeftBottomTangentX,
      "Left bottom tangent y": l10n.fxLeftBottomTangentY,
      "Left edge": l10n.fxLeftEdge,
      "Left top tangent x": l10n.fxLeftTopTangentX,
      "Left top tangent y": l10n.fxLeftTopTangentY,
      "LUT": l10n.fxLut,
      "Length": l10n.fxLength,
      "Lens": l10n.fxLens,
      "Lens distort": l10n.fxLensDistort,
      "Lens file": l10n.fxLensFile,
      "Lens flare": l10n.fxLensFlare,
      "Lens options": l10n.fxLensOptions,
      "Level": l10n.fxLevel,
      "Levels": l10n.fxLevels,
      "Lift": l10n.fxLift,
      "Light direction": l10n.fxLightDirection,
      "Light tint": l10n.fxLightTint,
      "Light wrap": l10n.fxLightWrap,
      "Light x": l10n.fxLightX,
      "Light y": l10n.fxLightY,
      "Lighten": l10n.fxLighten,
      "Lightness": l10n.fxLightness,
      "Lights": l10n.fxLights,
      "Line period": l10n.fxLinePeriod,
      "Linear": l10n.fxLinear,
      "Linear wipe": l10n.fxLinearWipe,
      "Low": l10n.fxLow,
      "Lower left x": l10n.fxLowerLeftX,
      "Lower left y": l10n.fxLowerLeftY,
      "Lower right x": l10n.fxLowerRightX,
      "Lower right y": l10n.fxLowerRightY,
      "Luminance": l10n.fxLuminance,
      "Luminance only": l10n.fxLuminanceOnly,
      "Magenta": l10n.fxMagenta,
      "Magentas": l10n.fxMagentas,
      "Manual": l10n.fxManual,
      "Manual light": l10n.fxManualLight,
      "Map black to": l10n.fxMapBlackTo,
      "Map white to": l10n.fxMapWhiteTo,
      "Master": l10n.fxMaster,
      "Matte": l10n.fxMatte,
      "Matte key": l10n.fxMatteKey,
      "Matte layer": l10n.fxMatteLayer,
      "Max ghosts": l10n.fxMaxGhosts,
      "Maximum signal": l10n.fxMaximumSignal,
      "Median": l10n.fxMedian,
      "Midtone contrast": l10n.fxMidtoneContrast,
      "Midtones": l10n.fxMidtones,
      "Mirror": l10n.fxMirror,
      "Mirror edges": l10n.fxMirrorEdges,
      "Mix": l10n.fxMix,
      "Mode": l10n.fxMode,
      "More options": l10n.fxMoreOptions,
      "Mosaic": l10n.fxMosaic,
      "Motion blur": l10n.fxMotionBlur,
      "Motion vectors": l10n.fxMotionVectors,
      "Multiply": l10n.fxMultiply,
      "Near blur": l10n.fxNearBlur,
      "Noise": l10n.fxNoise,
      "Noise type": l10n.fxNoiseType,
      "None": l10n.fxNone,
      "Normal": l10n.fxNormal,
      "NTSC": l10n.fxNtsc,
      "Offset": l10n.fxOffset,
      "Offset x": l10n.fxOffsetX,
      "Offset y": l10n.fxOffsetY,
      "Opacity": l10n.fxOpacity,
      "Opacity %": l10n.fxOpacityPct,
      "Operate on alpha": l10n.fxOperateOnAlpha,
      "Orange": l10n.fxOrange,
      "Orientation": l10n.fxOrientation,
      "Output black": l10n.fxOutputBlack,
      "Output height": l10n.fxOutputHeight,
      "Output white": l10n.fxOutputWhite,
      "Output width": l10n.fxOutputWidth,
      "Overlay": l10n.fxOverlay,
      "PAL": l10n.fxPal,
      "Per-axis wobble": l10n.fxPerAxisWobble,
      "Perlin": l10n.fxPerlin,
      "Phase": l10n.fxPhase,
      "Phase offset": l10n.fxPhaseOffset,
      "Photo filter": l10n.fxPhotoFilter,
      "Pinning": l10n.fxPinning,
      "Placement": l10n.fxPlacement,
      "Polar coordinates": l10n.fxPolarCoordinates,
      "Polar to rectangular": l10n.fxPolarToRectangular,
      "Position x": l10n.fxPositionX,
      "Position y": l10n.fxPositionY,
      "Posterize": l10n.fxPosterize,
      "Posterize time": l10n.fxPosterizeTime,
      "Preserve luminance": l10n.fxPreserveLuminance,
      "Preserve luminosity": l10n.fxPreserveLuminosity,
      "Quality": l10n.fxQuality,
      "Radial": l10n.fxRadial,
      "RGB split": l10n.fxRgbSplit,
      "Radial blur": l10n.fxRadialBlur,
      "Radial wipe": l10n.fxRadialWipe,
      "Radius": l10n.fxRadius,
      "Ramp": l10n.fxRamp,
      "Rec. 709": l10n.fxRec709,
      "Rectangular to polar": l10n.fxRectangularToPolar,
      "Red": l10n.fxRed,
      "Red blur": l10n.fxRedBlur,
      "Reds": l10n.fxReds,
      "Reduce brightness": l10n.fxReduceBrightness,
      "Reduce saturation": l10n.fxReduceSaturation,
      "Relief": l10n.fxRelief,
      "Remove edge leak": l10n.fxRemoveEdgeLeak,
      "Rendered": l10n.fxRendered,
      "Repeat": l10n.fxRepeat,
      "Repeat edge pixels": l10n.fxRepeatEdgePixels,
      "Replace colour": l10n.fxReplaceColour,
      "Replace method": l10n.fxReplaceMethod,
      "Reset interval": l10n.fxResetInterval,
      "Reverse": l10n.fxReverse,
      "Right bottom tangent x": l10n.fxRightBottomTangentX,
      "Right bottom tangent y": l10n.fxRightBottomTangentY,
      "Right edge": l10n.fxRightEdge,
      "Right top tangent x": l10n.fxRightTopTangentX,
      "Right top tangent y": l10n.fxRightTopTangentY,
      "Rim brightness": l10n.fxRimBrightness,
      "Ripple": l10n.fxRipple,
      "Rise": l10n.fxRise,
      "Roll speed": l10n.fxRollSpeed,
      "Rotation": l10n.fxRotation,
      "Rotation amount": l10n.fxRotationAmount,
      "Rotation frequency": l10n.fxRotationFrequency,
      "Rotation °": l10n.fxRotationDeg,
      "Roughen": l10n.fxRoughen,
      "Roughen edges": l10n.fxRoughenEdges,
      "Roundness": l10n.fxRoundness,
      "Rows/columns jitter": l10n.fxRowsColumnsJitter,
      "Samples": l10n.fxSamples,
      "Saturation": l10n.fxSaturation,
      "Sawtooth": l10n.fxSawtooth,
      "Scale": l10n.fxScale,
      "Scale height": l10n.fxScaleHeight,
      "Scale width": l10n.fxScaleWidth,
      "Scale x %": l10n.fxScaleXPct,
      "Scale y %": l10n.fxScaleYPct,
      "Scanlines": l10n.fxScanlines,
      "Scatter": l10n.fxScatter,
      "Screen": l10n.fxScreen,
      "Screen balance": l10n.fxScreenBalance,
      "Screen colour": l10n.fxScreenColour,
      "Screen gain": l10n.fxScreenGain,
      "Screen matte": l10n.fxScreenMatte,
      "Screen pre-blur": l10n.fxScreenPreBlur,
      "Screen shrink/grow": l10n.fxScreenShrinkGrow,
      "Screen softness": l10n.fxScreenSoftness,
      "Seed": l10n.fxSeed,
      "Sepia": l10n.fxSepia,
      "Set matte": l10n.fxSetMatte,
      "Shadow amount": l10n.fxShadowAmount,
      "Shadow colour": l10n.fxShadowColour,
      "Shadow highlight": l10n.fxShadowHighlight,
      "Shadow only": l10n.fxShadowOnly,
      "Shadow tonal width": l10n.fxShadowTonalWidth,
      "Shadows": l10n.fxShadows,
      "Shake": l10n.fxShake,
      "Shape": l10n.fxShape,
      "Sharp colours": l10n.fxSharpColours,
      "Sharpen": l10n.fxSharpen,
      "Shift x": l10n.fxShiftX,
      "Shift y": l10n.fxShiftY,
      "Show points": l10n.fxShowPoints,
      "Shutter": l10n.fxShutter,
      "Shutter angle": l10n.fxShutterAngle,
      "Shutter phase": l10n.fxShutterPhase,
      "Sine": l10n.fxSine,
      "Size": l10n.fxSize,
      "Slice repeat": l10n.fxSliceRepeat,
      "Soft colour": l10n.fxSoftColour,
      "Soft light": l10n.fxSoftLight,
      "Softness": l10n.fxSoftness,
      "Source": l10n.fxSource,
      "Source height": l10n.fxSourceHeight,
      "Source width": l10n.fxSourceWidth,
      "Spherize": l10n.fxSpherize,
      "Spiky": l10n.fxSpiky,
      "Spin": l10n.fxSpin,
      "Sprite flare": l10n.fxSpriteFlare,
      "Square": l10n.fxSquare,
      "Squeeze": l10n.fxSqueeze,
      "Standard": l10n.fxStandard,
      "Starburst intensity": l10n.fxStarburstIntensity,
      "Start angle": l10n.fxStartAngle,
      "Start colour": l10n.fxStartColour,
      "Start x": l10n.fxStartX,
      "Start y": l10n.fxStartY,
      "Status": l10n.fxStatus,
      "Stops": l10n.fxStops,
      "Streak": l10n.fxStreak,
      "Streak angle": l10n.fxStreakAngle,
      "Streak intensity": l10n.fxStreakIntensity,
      "Streak length": l10n.fxStreakLength,
      "Stretch": l10n.fxStretch,
      "Strobe": l10n.fxStrobe,
      "Style": l10n.fxStyle,
      "Stylise": l10n.fxStylise,
      "Sub influence": l10n.fxSubInfluence,
      "Sub scaling": l10n.fxSubScaling,
      "Sub settings": l10n.fxSubSettings,
      "Subtract": l10n.fxSubtract,
      "Symmetric": l10n.fxSymmetric,
      "Temperature": l10n.fxTemperature,
      "Temporal": l10n.fxTemporal,
      "Texture": l10n.fxTexture,
      "Texture contrast": l10n.fxTextureContrast,
      "Texturize": l10n.fxTexturize,
      "Threshold": l10n.fxThreshold,
      "Threshold softness": l10n.fxThresholdSoftness,
      "Tile": l10n.fxTile,
      "Tile centre x": l10n.fxTileCentreX,
      "Tile centre y": l10n.fxTileCentreY,
      "Tile height": l10n.fxTileHeight,
      "Tile width": l10n.fxTileWidth,
      "Tint": l10n.fxTint,
      "Tint colour": l10n.fxTintColour,
      "Top and bottom": l10n.fxTopAndBottom,
      "Top edge": l10n.fxTopEdge,
      "Top left tangent x": l10n.fxTopLeftTangentX,
      "Top left tangent y": l10n.fxTopLeftTangentY,
      "Top right tangent x": l10n.fxTopRightTangentX,
      "Top right tangent y": l10n.fxTopRightTangentY,
      "Transform": l10n.fxTransform,
      "Transition": l10n.fxTransition,
      "Transparent": l10n.fxTransparent,
      "Triangle": l10n.fxTriangle,
      "Trigger": l10n.fxTrigger,
      "Tritone": l10n.fxTritone,
      "Turbulent": l10n.fxTurbulent,
      "Turbulent displace": l10n.fxTurbulentDisplace,
      "Twirl": l10n.fxTwirl,
      "Twist": l10n.fxTwist,
      "Type": l10n.fxType,
      "Ultra": l10n.fxUltra,
      "Underwater": l10n.fxUnderwater,
      "Uniform": l10n.fxUniform,
      "Uniform scaling": l10n.fxUniformScaling,
      "Unsharp mask": l10n.fxUnsharpMask,
      "Upper left x": l10n.fxUpperLeftX,
      "Upper left y": l10n.fxUpperLeftY,
      "Upper right x": l10n.fxUpperRightX,
      "Upper right y": l10n.fxUpperRightY,
      "Use focus point": l10n.fxUseFocusPoint,
      "Use inner radius": l10n.fxUseInnerRadius,
      "Use masks": l10n.fxUseMasks,
      "Use source colour": l10n.fxUseSourceColour,
      "Utility": l10n.fxUtility,
      "Value": l10n.fxValue,
      "Vector scale": l10n.fxVectorScale,
      "Vegas": l10n.fxVegas,
      "Venetian blinds": l10n.fxVenetianBlinds,
      "Vertical": l10n.fxVertical,
      "Vertical amount": l10n.fxVerticalAmount,
      "Vertical axis": l10n.fxVerticalAxis,
      "Vertical blocks": l10n.fxVerticalBlocks,
      "Vertical channel": l10n.fxVerticalChannel,
      "Vertical distortion": l10n.fxVerticalDistortion,
      "Vibrancy": l10n.fxVibrancy,
      "View": l10n.fxView,
      "Vignette": l10n.fxVignette,
      "Violet": l10n.fxViolet,
      "Vivid light": l10n.blendVividLight,
      "Warming filter (81)": l10n.fxWarmingFilter81,
      "Warming filter (85)": l10n.fxWarmingFilter85,
      "Warming filter (LBA)": l10n.fxWarmingFilterLba,
      "Warp": l10n.fxWarp,
      "Wave": l10n.fxWave,
      "Wave height": l10n.fxWaveHeight,
      "Wave type": l10n.fxWaveType,
      "Wave warp": l10n.fxWaveWarp,
      "Wave width": l10n.fxWaveWidth,
      "Wavelength": l10n.fxWavelength,
      "Width": l10n.fxWidth,
      "Wiggle type": l10n.fxWiggleType,
      "Wiggles per second": l10n.fxWigglesPerSecond,
      "Wiggly": l10n.fxWiggly,
      "Wipe": l10n.fxWipe,
      "Wipe angle": l10n.fxWipeAngle,
      "Wipe centre x": l10n.fxWipeCentreX,
      "Wipe centre y": l10n.fxWipeCentreY,
      "X amount": l10n.fxXAmount,
      "X frequency": l10n.fxXFrequency,
      "Y amount": l10n.fxYAmount,
      "Y frequency": l10n.fxYFrequency,
      "Yellow": l10n.fxYellow,
      "Yellows": l10n.fxYellows,
      "Z amount": l10n.fxZAmount,
      "Z frequency": l10n.fxZFrequency,
      "Zoom": l10n.fxZoom,
      // Settings -> Keymap: what each shortcut does, and the headings saying
      // where it is live (crates/lumit-keymap/src/lib.rs).
      "Play or pause": l10n.keyPlayOrPause,
      "Shuttle backwards": l10n.keyShuttleBackwards,
      "Shuttle pause": l10n.keyShuttlePause,
      "Shuttle forwards": l10n.keyShuttleForwards,
      "Next frame": l10n.keyNextFrame,
      "Previous frame": l10n.keyPreviousFrame,
      "Forward ten frames": l10n.keyForwardTenFrames,
      "Back ten frames": l10n.keyBackTenFrames,
      "Go to the start": l10n.keyGoToTheStart,
      "Go to the end": l10n.keyGoToTheEnd,
      "Go to work-area start": l10n.keyGoToWorkAreaStart,
      "Go to work-area end": l10n.keyGoToWorkAreaEnd,
      "Go to the layer's in point": l10n.keyGoToTheLayerSInPoint,
      "Go to the layer's out point": l10n.keyGoToTheLayerSOutPoint,
      "Previous keyframe": l10n.keyPreviousKeyframe,
      "Next keyframe": l10n.keyNextKeyframe,
      "Previous edit point": l10n.keyPreviousEditPoint,
      "Next edit point": l10n.keyNextEditPoint,
      "Set work-area start to the playhead":
          l10n.keySetWorkAreaStartToThePlayhead,
      "Set work-area end to the playhead": l10n.keySetWorkAreaEndToThePlayhead,
      "Add a marker at the playhead": l10n.keyAddAMarkerAtThePlayhead,
      "Delete the selection": l10n.keyDeleteTheSelection,
      "Open the command palette": l10n.keyOpenTheCommandPalette,
      "Open the FX console": l10n.keyOpenTheFxConsole,
      "Add to the export queue": l10n.keyAddToTheExportQueue,
      "Composition settings": l10n.keyCompositionSettings,
      "Undo": l10n.keyUndo,
      "Redo": l10n.keyRedo,
      "Select every layer": l10n.keySelectEveryLayer,
      "Deselect everything": l10n.keyDeselectEverything,
      "New project": l10n.keyNewProject,
      "Open a project": l10n.keyOpenAProject,
      "Save the project": l10n.keySaveTheProject,
      "Save the project somewhere else": l10n.keySaveTheProjectSomewhereElse,
      "Import footage": l10n.keyImportFootage,
      "Export the composition": l10n.keyExportTheComposition,
      "New composition": l10n.keyNewComposition,
      "Open Settings": l10n.keyOpenSettings,
      "Open Project settings": l10n.keyOpenProjectSettings,
      "Maximise the panel under the pointer":
          l10n.keyMaximiseThePanelUnderThePointer,
      "Show or hide the graph editor": l10n.keyShowOrHideTheGraphEditor,
      "Selection tool": l10n.keySelectionTool,
      "Hand tool": l10n.keyHandTool,
      "Zoom tool": l10n.keyZoomTool,
      "Anchor point tool": l10n.keyAnchorPointTool,
      "Razor tool": l10n.keyRazorTool,
      "Shape tool": l10n.keyShapeTool,
      "Pen tool": l10n.keyPenTool,
      "Rotation tool": l10n.keyRotationTool,
      "Type tool": l10n.keyTypeTool,
      "Brush tool": l10n.keyBrushTool,
      "Roto brush tool": l10n.keyRotoBrushTool,
      "Puppet tool": l10n.keyPuppetTool,
      "Camera tool": l10n.keyCameraTool,
      "Reveal Position": l10n.keyRevealPosition,
      "Reveal Scale": l10n.keyRevealScale,
      "Reveal Rotation": l10n.keyRevealRotation,
      "Reveal Opacity": l10n.keyRevealOpacity,
      "Reveal Anchor point": l10n.keyRevealAnchorPoint,
      "Reveal Effects": l10n.keyRevealEffects,
      "Reveal Masks": l10n.keyRevealMasks,
      "Reveal animated properties": l10n.keyRevealAnimatedProperties,
      "Reveal Volume": l10n.keyRevealVolume,
      "Reveal Audio, again for the waveform":
          l10n.keyRevealAudioAgainForTheWaveform,
      "Move the layer's in point to the playhead":
          l10n.keyMoveTheLayerSInPointToThePlayhead,
      "Move the layer's out point to the playhead":
          l10n.keyMoveTheLayerSOutPointToThePlayhead,
      "Trim the layer's in point to the playhead":
          l10n.keyTrimTheLayerSInPointToThePlayhead,
      "Trim the layer's out point to the playhead":
          l10n.keyTrimTheLayerSOutPointToThePlayhead,
      "Split the layer at the playhead": l10n.keySplitTheLayerAtThePlayhead,
      "Duplicate the layer": l10n.keyDuplicateTheLayer,
      "Pre-compose the layer": l10n.keyPreComposeTheLayer,
      "Give the layer a Retime": l10n.keyGiveTheLayerARetime,
      "Zoom in": l10n.keyZoomIn,
      "Zoom out": l10n.keyZoomOut,
      "Zoom to fit": l10n.keyZoomToFit,
      "Rename the layer": l10n.keyRenameTheLayer,
      "Rename the selected item": l10n.keyRenameTheSelectedItem,
      "Rename the selected effect": l10n.keyRenameTheSelectedEffect,
      "Show or hide the layer": l10n.keyShowOrHideTheLayer,
      "Easy ease": l10n.keyEasyEase,
      "Easy ease in": l10n.keyEasyEaseIn,
      "Easy ease out": l10n.keyEasyEaseOut,
      "Fit the curves to the pane": l10n.keyFitTheCurvesToThePane,
      "Fit the picture to the panel": l10n.keyFitThePictureToThePanel,
      "Full resolution": l10n.keyFullResolution,
      "Half resolution": l10n.keyHalfResolution,
      "Quarter resolution": l10n.keyQuarterResolution,
      "Show or hide the rulers": l10n.keyShowOrHideTheRulers,
      "Show or hide the grid": l10n.keyShowOrHideTheGrid,
      "Focus the next panel": l10n.keyFocusTheNextPanel,
      "Focus the previous panel": l10n.keyFocusThePreviousPanel,
      "Focus the panel's search box": l10n.keyFocusThePanelSSearchBox,
      "Anywhere": l10n.keyAnywhere,
      "Tools": l10n.keyTools,
      "Project panel": l10n.keyProjectPanel,
      "Timeline": l10n.keyTimeline,
      "Viewer": l10n.keyViewer,
      "Graph editor": l10n.keyGraphEditor,
      "Panels": l10n.keyPanels,
      "Effect controls": l10n.keyEffectControls,
      "Copy the selection": l10n.keyCopyTheSelection,
      "Cut the selection": l10n.keyCutTheSelection,
      "Paste": l10n.keyPaste,
    };

// --- The import report's reasons (K-303, docs/11 §9) ----------------------
//
// The other way engine text is translated, and the one docs/17 prescribes for
// a sentence with a fact in it: "blend mode Dissolve has no equivalent —
// imported as Normal" is a different whole text for every blend mode, so the
// table above — which looks a label up by its English — could never hold it.
//
// So the *pieces* cross instead. `lumit_import::Reason::key` sends a stable id
// (`blend_mode_unavailable`) and `::args` sends the blanks by name
// (`ae_mode: "Dissolve"`), and the sentence is written here, in the reader's
// language. `test/l10n/engine_labels_test.dart` reads the Rust enum and fails
// if a variant has no case below, so a reason added to the engine cannot
// quietly ship as English.

/// The one-line reason for an import report row, or null when this build has
/// no sentence for [key] — the caller shows the engine's own English instead,
/// the same courtesy [engineLabel] extends to a label it has never seen.
///
/// Fact values go through [engineLabel] on the way in: most are After Effects'
/// own words or plain numbers and pass through unchanged, while the few that
/// are Lumit's own — the effect or feature an approximation landed on — are
/// translated if anyone has translated them.
String? importReason(String key, Map<String, String> args) {
  String a(String name) => engineLabel(args[name] ?? '');
  switch (key) {
    // Items and compositions.
    case 'item_unreadable':
      return l10n.aeItemUnreadable;
    case 'comp_missing':
      return l10n.aeCompMissing;
    case 'comp_frame_rate_guessed':
      return l10n.aeCompFrameRateGuessed(a('used'));
    case 'comp_duration_guessed':
      return l10n.aeCompDurationGuessed(a('used'));
    case 'pixel_aspect_ignored':
      return l10n.aePixelAspectIgnored(a('par'));
    case 'comp_start_ignored':
      return l10n.aeCompStartIgnored(a('start'));
    case 'nested_preserve_ignored':
      final fps = args['fps'] == 'true';
      final resolution = args['resolution'] == 'true';
      if (fps && resolution) return l10n.aeNestedPreserveBoth;
      return fps ? l10n.aeNestedPreserveRate : l10n.aeNestedPreserveResolution;
    case 'project_blending_differs':
      return l10n.aeProjectBlendingDiffers(a('bits'));
    case 'renderer_unrecognised':
      return l10n.aeRendererUnrecognised(a('renderer'));
    case 'media_missing':
      return l10n.aeMediaMissing(a('path'));
    case 'media_not_found':
      return l10n.aeMediaNotFound;
    case 'media_placeholder':
      return l10n.aeMediaPlaceholder;

    // Layers.
    case 'layer_unreadable':
      return l10n.aeLayerUnreadable;
    case 'layer_kind_unsupported':
      return l10n.aeLayerKindUnsupported(a('ae_kind'));
    case 'layer_source_missing':
      return l10n.aeLayerSourceMissing(a('id'));
    case 'layer_span_repaired':
      return l10n.aeLayerSpanRepaired;
    case 'audio_layer_as_footage':
      return l10n.aeAudioLayerAsFootage;
    case 'audio_levels_differ':
      return l10n.aeAudioLevelsDiffer(a('left'), a('right'));
    case 'guide_layer_not_supported':
      return l10n.aeGuideLayerNotSupported;
    case 'preserve_transparency_not_supported':
      return l10n.aePreserveTransparencyNotSupported;
    case 'layer_quality_ignored':
      return l10n.aeLayerQualityIgnored(a('quality'));
    case 'stretch_as_retime':
      return l10n.aeStretchAsRetime(a('percent'));
    case 'flow_engine_differs':
      return l10n.aeFlowEngineDiffers;
    case 'parent_missing':
      return l10n.aeParentMissing(a('index'));
    case 'matte_target_missing':
      return l10n.aeMatteTargetMissing(a('index'));
    case 'blend_mode_unavailable':
      return l10n.aeBlendModeUnavailable(a('ae_mode'));
    case 'blend_mode_classic':
      return l10n.aeBlendModeClassic(a('ae_mode'));
    case 'shape_contents_not_mapped':
      return l10n.aeShapeContentsNotMapped;
    case 'text_styling_not_mapped':
      return l10n.aeTextStylingNotMapped;
    case 'light_kind_approximated':
      return l10n.aeLightKindApproximated(a('ae_kind'));

    // Properties and keyframes.
    case 'spatial_tangents_flattened':
      return l10n.aeSpatialTangentsFlattened;
    case 'expression_carried':
      return l10n.aeExpressionCarried;
    case 'expression_disabled_carried':
      return l10n.aeExpressionDisabledCarried;
    case 'property_unreadable':
      return l10n.aePropertyUnreadable(a('match_name'));
    case 'chunk_unreadable':
      return l10n.aeChunkUnreadable(a('chunk'));

    // Masks.
    case 'mask_feather_axes_differ':
      return l10n.aeMaskFeatherAxesDiffer(a('x'), a('y'));
    case 'mask_roto_bezier_flattened':
      return l10n.aeMaskRotoBezierFlattened;

    // Effects.
    case 'effect_placeholder':
      return l10n.aeEffectPlaceholder(a('match_name'));
    case 'effect_params_unreadable':
      return l10n.aeEffectParamsUnreadable(a('count'));
    case 'effect_param_not_carried':
      return l10n.aeEffectParamNotCarried(a('effect'), a('param'));
    case 'effect_param_approximated':
      return l10n.aeEffectParamApproximated(
          a('effect'), a('param'), a('imported_as'));
    case 'effect_differs':
      return l10n.aeEffectDiffers(a('effect'), a('detail'));
    case 'effect_speed_as_keyframes':
      return l10n.aeEffectSpeedAsKeyframes(a('effect'), a('param'));
    case 'effect_suggestion':
      return l10n.aeEffectSuggestion(a('match_name'), a('instead'));
    case 'effect_param_rebased':
      return l10n.aeEffectParamRebased(a('effect'), a('param'));

    default:
      return null;
  }
}

/// Whether this build has a sentence for [key] — what the sync test asserts.
bool hasImportReason(String key) => importReason(key, const {}) != null;
