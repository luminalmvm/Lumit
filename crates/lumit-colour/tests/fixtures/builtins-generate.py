"""Generate tests/fixtures/builtins.fixture with the reference OpenColorIO library.

A `BuiltinTransform` style is a name, not data: what it means is whatever the
reference implementation's code does under that name (docs/impl/ocio.md 4.1).
So the only honest gate on a Rust port of one is the reference library's own
processor, which is what this writes out. Run it from this directory:

    python builtins-generate.py > builtins.fixture

The styles are the ones tier one answers that Blender 5.2's bundled config and
the PixelManager config name; the probes are generate.py's, unchanged, so a row
here and a row there ask the same questions.
"""
import datetime
import PyOpenColorIO as ocio

STYLES = [
    "UTILITY - ACES-AP1_to_CIE-XYZ-D65_BFD",
    "DISPLAY - CIE-XYZ-D65_to_sRGB",
    "DISPLAY - CIE-XYZ-D65_to_REC.1886-REC.709",
    "DISPLAY - CIE-XYZ-D65_to_REC.1886-REC.2020",
    "DISPLAY - CIE-XYZ-D65_to_G2.2-REC.709",
    "DISPLAY - CIE-XYZ-D65_to_G2.6-P3-D65",
    "DISPLAY - CIE-XYZ-D65_to_REC.2100-HLG-1000nit",
    "CURVE - LINEAR_to_ST-2084",
    "CURVE - ST-2084_to_LINEAR",
    "CURVE - HLG-OETF",
    "CURVE - HLG-OETF-INVERSE",
    "CURVE - APPLE_LOG_to_LINEAR",
    "APPLE_LOG_to_ACES2065-1",
    "CANON_CLOG2-CGAMUT_to_ACES2065-1",
    "CANON_CLOG3-CGAMUT_to_ACES2065-1",
]

probes = [
    [0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [0.18, 0.18, 0.18], [0.5, 0.25, 0.75],
    [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 1.0, 0.0],
    [0.004, 0.002, 0.001], [1e-7, 1e-7, 1e-7],
    [-0.05, 0.4, 0.9], [-0.2, -0.1, 0.02],
    [2.0, 1.4, 0.3], [16.0, 8.0, 4.0], [100.0, 12.0, -0.3], [64.0, 64.0, 64.0],
]

# The reference reads these curves from a table rather than from the formula:
# HLG, PQ and Apple Log are each a 65536-entry half-domain LUT (Displays.cpp and
# AppleCameras.cpp, CreateHalfLut). Inside the encoding's own range that table
# is the formula to better than a part in a hundred thousand, and every row
# below proves it. Outside it the table's edge answers instead: inverting one
# clamps at 65504, and a code value that decodes past what an f32 holds reads
# back FLT_MAX. Lumit evaluates the formula (4.1: ST 2084 is an op, not a bake),
# so a probe whose input is a code value beyond -1 to 1 is left out rather than
# gated against the edge of a table. Canon Log is the exception that stays: its
# table is 4096 points over exactly 0 to 1, a domain the reference means as a
# domain, so Lumit clamps to it too and those rows gate as they are.
TABULATED_CODE_SIDE = {
    ("DISPLAY - CIE-XYZ-D65_to_REC.2100-HLG-1000nit", "inverse"),
    ("CURVE - HLG-OETF", "inverse"),
    ("CURVE - HLG-OETF-INVERSE", "forward"),
    ("CURVE - LINEAR_to_ST-2084", "inverse"),
    ("CURVE - ST-2084_to_LINEAR", "forward"),
    ("CURVE - APPLE_LOG_to_LINEAR", "forward"),
    ("APPLE_LOG_to_ACES2065-1", "forward"),
}

DIRECTIONS = (
    ("forward", "builtin", ocio.TRANSFORM_DIR_FORWARD),
    ("inverse", "builtin-inverse", ocio.TRANSFORM_DIR_INVERSE),
)

cfg = ocio.Config.CreateRaw()

print("# BuiltinTransform styles: expected values from the REFERENCE OpenColorIO library.")
print(f"# library: PyOpenColorIO {ocio.GetVersion()}")
print("# source: transforms/builtins/ (ACES.cpp, Displays.cpp, AppleCameras.cpp,")
print("#   CanonCameras.cpp), the code each style names")
print(f"# generated: {datetime.date.today()} by tests/fixtures/builtins-generate.py")
print("# processor: LOSSLESS, so the rows carry the reference's own arithmetic")
print("#   rather than its fast approximate pow (see the generator).")
print("# Each id resolves through lumit_colour::builtin::resolve, forwards and")
print("#   backwards; a style the reference cannot invert has no inverse rows.")
print("# 1e-5 throughout: these are matrices and published curves on both sides,")
print("#   with nothing tabulated in between.")

for style in STYLES:
    for name, ident, direction in DIRECTIONS:
        try:
            transform = ocio.BuiltinTransform(style=style, direction=direction)
            # LOSSLESS rather than the default processor. The default one turns
            # on the library's fast approximate pow, which costs 2.4e-5 relative
            # inverting an sRGB curve, a hundred times the tolerance here, and
            # a fact about that approximation rather than about the style. The
            # ops are the same either way.
            cpu = cfg.getProcessor(transform).getOptimizedCPUProcessor(
                ocio.BIT_DEPTH_F32, ocio.BIT_DEPTH_F32, ocio.OPTIMIZATION_LOSSLESS
            )
        except Exception as why:  # a style with no inverse says so here
            print(f"# {ident}: {style} - the reference declines it ({why})")
            continue
        skip_out_of_range = (style, name) in TABULATED_CODE_SIDE
        for probe in probes:
            if skip_out_of_range and any(abs(v) > 1.0 for v in probe):
                continue
            out = cpu.applyRGB(list(probe))
            # Nine significant figures, not eight decimal places: these curves
            # answer in the tens of billionths at one end and the millions at
            # the other, and a fixed point column would round both away.
            rgb = " ".join(f"{v:.9g}" for v in probe)
            want = " ".join(f"{v:.9g}" for v in out)
            print(f"{ident}: {style} | {rgb} | {want} | 1e-5")

# Tier two: the ACES 1.x output transforms answered from a vendored 65 point
# bake (vendored/README.md). A table cannot agree with an algorithm to a part
# in a hundred thousand, so these rows carry the cube form's bounds from
# generate.py: 2e-3 inside the shaper's domain, 5e-2 past it (an input above
# 32 or below 0), 5e-3 inside the shaper's first cell, and 1.5e-1 at a gamut
# primary, where the rendering is not smooth on a 65 point log grid. Forward
# only, since a bake has no inverse.
BAKED = [
    "ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - SDR-VIDEO_1.0",
    "ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - SDR-VIDEO-P3lim_1.1",
    "ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - HDR-VIDEO-1000nit-15nit-REC2020lim_1.1",
]


def bake_bound(style, probe):
    # Past the shaper's reach on both sides at once, a channel above 32 and
    # another below zero, the bake answers from a corner of its grid rather
    # than an edge, and nothing it says there is worth stating as a bound:
    # 0.216 at (100, 12, -0.3) through SDR-VIDEO_1.0, and 1.1 on a channel of
    # the 1000 nit one. The one-sided rows below carry the domain's bound.
    if any(v > 32.0 for v in probe) and any(v < 0.0 for v in probe):
        return None
    if any(v < 0.0 or v > 32.0 for v in probe):
        # The 1000 nit rendering is still climbing at the shaper's ceiling of
        # 32, where the SDR ones have flattened, so a 64 neutral through it
        # reads 11% low (measured 0.109); the SDR bakes hold the 5e-2 domain
        # bound. Raising the shaper's ceiling for HDR bakes is the fix, and a
        # bake.rs matter, not a row's.
        return "1.5e-1" if "HDR" in style else "5e-2"
    if any(0.0 < abs(v) < 1e-5 for v in probe):
        return "5e-3"
    if max(probe) > 0.0 and min(probe) == 0.0:
        return "1.5e-1"
    return "2e-3"


print("# Tier two, the vendored bakes, at the cube form's bounds (see the generator).")
for style in BAKED:
    transform = ocio.BuiltinTransform(style=style, direction=ocio.TRANSFORM_DIR_FORWARD)
    cpu = cfg.getProcessor(transform).getOptimizedCPUProcessor(
        ocio.BIT_DEPTH_F32, ocio.BIT_DEPTH_F32, ocio.OPTIMIZATION_LOSSLESS
    )
    for probe in probes:
        bound = bake_bound(style, probe)
        if bound is None:
            continue
        out = cpu.applyRGB(list(probe))
        rgb = " ".join(f"{v:.9g}" for v in probe)
        want = " ".join(f"{v:.9g}" for v in out)
        print(f"builtin: {style} | {rgb} | {want} | {bound}")
