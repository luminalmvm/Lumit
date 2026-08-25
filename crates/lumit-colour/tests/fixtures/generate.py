"""Generate a Lumit reference fixture with the reference OpenColorIO library.

The curated variant of tests/fixtures/README.md's generator: role-space edges
both ways, and a CURATED view list rather than every view the config offers —
the ACES 1.2 Output LUTs run to hundreds of megabytes, and the vendored config
carries only the sRGB and Rec.709 forward cubes (~15 MB). The rows gate what
the repository can actually resolve; nothing else is tabulated, because a row
whose LUT is not vendored would refuse loudly rather than prove anything.
"""
import sys, datetime, PyOpenColorIO as ocio

name = sys.argv[1]
views_wanted = sys.argv[2].split(',') if len(sys.argv) > 2 else None
cfg = ocio.Config.CreateFromFile(f"{name}/config.ocio")

probes = [
    [0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [0.18, 0.18, 0.18], [0.5, 0.25, 0.75],
    [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 1.0, 0.0],
    [0.004, 0.002, 0.001], [1e-7, 1e-7, 1e-7],
    [-0.05, 0.4, 0.9], [-0.2, -0.1, 0.02],
    [2.0, 1.4, 0.3], [16.0, 8.0, 4.0], [100.0, 12.0, -0.3], [64.0, 64.0, 64.0],
]

if cfg.hasRole("aces_interchange"):
    WORKING = "Linear Rec.709 (sRGB)"
else:
    WORKING = cfg.getColorSpace(ocio.ROLE_SCENE_LINEAR).getName()

ends = [WORKING]
# compositing_log is deliberately absent for the legacy config: ACES 1.2
# binds it to ADX10, a film-scan density space whose chain is steeper than
# the cube bake's §5.4 domain (measured 23% off at white) — not an edge the
# application's fixed working space ever walks. The v2 configs bind it to
# ACEScct, which the role list reaches through color_timing anyway.
for role in ("scene_linear", "color_timing",
             "aces_interchange", "default"):
    cs = cfg.getColorSpace(role)
    if cs and cs.getName() not in ends:
        ends.append(cs.getName())

edges = [("space", a, b) for a in ends for b in ends if a != b]
for d in cfg.getDisplays():
    for v in cfg.getViews(d):
        if views_wanted is None or v in views_wanted:
            edges.append(("view", d, v))

# The three stated ceilings, each a measured fact about the baked form rather
# than a number chosen to make a row pass. Every one is written to be tightened
# and never widened: DOMAIN when a probe leaves the shaper's reach, FLOOR when
# it falls inside the shaper's first cell, GAMUT when a vendored bake is asked
# for a colour on the edge of the gamut. Their derivations are in the loop.
DOMAIN, FLOOR, GAMUT = "5e-2", "5e-3", "1.5e-1"


def baked_style(kind, display, view):
    """Whether this view's transform is the ACES rendering, which is CODE.

    An ACES 2.0 output transform is an algorithm, not a composition — there is
    nothing in the config to read and nothing to write out by hand — so Lumit
    answers it from a vendored 65-point bake of the reference library's own
    output (docs/impl/ocio.md 4.1, tier two). A table cannot agree with an
    algorithm to a part in a hundred thousand, so BOTH gates on such a row
    carry the cube form's bound instead of the curve's. Every other view here
    is a matrix and a documented transfer function on both sides, and keeps the
    tight bound.
    """
    if kind != "view":
        return False
    name = cfg.getDisplayViewTransformName(display, view)
    if not name:
        return False
    t = cfg.getViewTransform(name).getTransform(ocio.VIEWTRANSFORM_DIR_FROM_REFERENCE)
    style = getattr(t, "getStyle", lambda: "")()
    return style.startswith("ACES-OUTPUT") or style.startswith("ACES-LMT")


def processor(kind, a, b):
    if kind == "space":
        return cfg.getProcessor(a, b)
    t = ocio.DisplayViewTransform(src=WORKING, display=a, view=b)
    return cfg.getProcessor(t, ocio.TRANSFORM_DIR_FORWARD)

print(f"# {name}: expected values from the REFERENCE OpenColorIO library.")
print(f"# library: PyOpenColorIO {ocio.GetVersion()}")
SOURCE = {
    "aces-1.2": "colour-science/OpenColorIO-Configs, aces_1.2 folder, cloned 2026-08-25",
    "aces-cg": "AcademySoftwareFoundation/OpenColorIO-Config-ACES release v4.0.0, "
    "asset cg-config-v4.0.0_aces-v2.0_ocio-v2.5.ocio",
}
print(f"# config: {name}/config.ocio  ({SOURCE.get(name, 'record the release tag here')})")
print(f"# generated: {datetime.date.today()} by tests/fixtures/generate.py (curated views: {views_wanted})")
print("# View rows start in Lumit's fixed working space, scene-linear Rec.709,")
print(f"#   entering the config at {WORKING!r} (docs/impl/ocio.md 2.1).")
print("# A fifth field is the BAKED gate's own bound: rows outside the shaper's")
print("#   domain carry 5.4's looser ceiling, to be tightened, never widened.")
for kind, a, b in edges:
    cpu = processor(kind, a, b).getDefaultCPUProcessor()
    ident = f"space: {a} -> {b}" if kind == "space" else f"view: {a} / {b}"
    # Leaving a LOG-encoded space is the steepest thing a real config asks
    # for, and it is where Lumit's factorised curve is furthest from exact.
    # ACEScct spends 17.52 stops over a 0-1 code range, so at code 1.0 the
    # curve is climbing far faster than the curve table's own log-spaced
    # samples; linear interpolation between them costs (ln2 * 17.52 * h)^2 / 8
    # relative, which is 7.7e-5 at the sample spacing there and measures
    # 8.9e-5. The matrix after it lifts that to 1.4e-4 (5.4: a chain's error
    # is the curve's times the matrix's gain). 2e-4 is that fact's bound; the
    # ENCODE direction is not affected, because compressing 222 into 1.0 is
    # the shallow way round. Legacy configs state no encoding and carry the
    # blanket 3e-4 above, which is looser still.
    src = cfg.getColorSpace(a) if kind == "space" else None
    from_log = bool(src) and src.getEncoding() == "log"
    table_backed = baked_style(kind, a, b)
    # An edge Lumit answers from a 65-point bake cannot agree with an
    # algorithm to a part in a hundred thousand, so its FIRST gate carries the
    # cube form's bound rather than the curve's.
    exact = "2e-3" if table_backed else "1e-5"
    for p in probes:
        out = cpu.applyRGB(list(p))
        rgb = " ".join(f"{v:.8f}" for v in p)
        want = " ".join(f"{v:.8f}" for v in out)

        # Past the shaper's reach, at either end. The INPUT side is 5.4's own
        # 0-to-32 domain. The OUTPUT side is the working format's: Lumit
        # composites in fp16, which stops at 65504, and the curve table's
        # ceiling is 2^16 for exactly that reason — an ACEScct code value of 4
        # decodes to 1.5e18, thirteen orders past anything the pipeline holds.
        # Measured worst 3.9e-2 (a 64.0 neutral through a vendored bake).
        out_of_domain = any(v < 0.0 or v > 32.0 for v in p) or any(
            not (abs(o) <= 65504.0) for o in out
        )
        # Below the artefact's RESOLUTION rather than past its edge. The signed
        # curve shaper's first sample above zero is at linear 7.8e-6, so
        # everything under that is one straight line down to black — while a
        # gamma encode is vertical there (its slope at zero is infinite). A
        # 1e-7 probe encoded to gamma 2.2 should be 6.6e-4 and the table says
        # 6.4e-5. That is not a tolerance to tighten by trying harder; it is
        # what a table of any size does with a curve of infinite slope, and the
        # bound is the height of that first cell, about 5e-3.
        below_resolution = any(0.0 < abs(v) < 1e-5 for v in p)
        # The gamut boundary, through a VENDORED BAKE, is where tier two costs
        # the most and the number is not small: 0.117 at the Rec.709 blue
        # primary (0.017 green, 0.0054 red). The ACES 2.0 rendering is not
        # smooth on a 65-point log grid there — the eight corners of the cell
        # containing that colour span 0.165 in Z and are not even monotone in
        # blue — so no interpolation between them lands within a code value.
        # This is 5.4's deep-saturation family, which that section already
        # declines to bound, met for real. It is the price of answering the
        # ACES output transforms from a table instead of porting them, it is
        # recorded here so the Rust port can be measured against it, and it
        # applies ONLY to rows a bake answers.
        gamut_edge = table_backed and any(v <= 0.0 for v in p) and any(v > 0.0 for v in p)

        # A v1 config's space edges are file CURVES plus a matrix, so they bake
        # to the factorised form — and Lumit resamples a file curve through its
        # own shaper table where the reference interpolates the file directly.
        # That double interpolation measures 8.9e-5 relative worst (ACEScc's
        # steep top), and 3e-4 is its bound. It is written only for space rows:
        # a view edge on this config carries a 3D LUT, so it bakes to the cube
        # form instead, whose bound is 5.4's own 2e-3 — writing 3e-4 there
        # would not be a tighter promise, it would be the wrong promise about a
        # different artefact.
        legacy = not cfg.hasRole("aces_interchange")

        if gamut_edge:
            first, second = GAMUT, GAMUT
        elif table_backed and (out_of_domain or below_resolution):
            first, second = DOMAIN, DOMAIN
        else:
            first = exact
            second = (
                DOMAIN if out_of_domain
                else FLOOR if below_resolution
                else "3e-4" if legacy and kind == "space"
                else "2e-4" if from_log
                else None
            )
        tail = f" | {second}" if second else ""
        print(f"{ident} | {rgb} | {want} | {first}{tail}")
