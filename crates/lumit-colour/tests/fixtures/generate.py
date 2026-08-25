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

def processor(kind, a, b):
    if kind == "space":
        return cfg.getProcessor(a, b)
    t = ocio.DisplayViewTransform(src=WORKING, display=a, view=b)
    return cfg.getProcessor(t, ocio.TRANSFORM_DIR_FORWARD)

print(f"# {name}: expected values from the REFERENCE OpenColorIO library.")
print(f"# library: PyOpenColorIO {ocio.GetVersion()}")
print(f"# config: {name}/config.ocio  (colour-science/OpenColorIO-Configs aces_1.2, cloned 2026-08-25)")
print(f"# generated: {datetime.date.today()} by tests/fixtures/generate.py (curated views: {views_wanted})")
print("# View rows start in Lumit's fixed working space, scene-linear Rec.709,")
print(f"#   entering the config at {WORKING!r} (docs/impl/ocio.md 2.1).")
print("# A fifth field is the BAKED gate's own bound: rows outside the shaper's")
print("#   domain carry 5.4's looser ceiling, to be tightened, never widened.")
for kind, a, b in edges:
    cpu = processor(kind, a, b).getDefaultCPUProcessor()
    ident = f"space: {a} -> {b}" if kind == "space" else f"view: {a} / {b}"
    for p in probes:
        out = cpu.applyRGB(list(p))
        rgb = " ".join(f"{v:.8f}" for v in p)
        want = " ".join(f"{v:.8f}" for v in out)
        out_of_domain = any(v < 0.0 or v > 32.0 for v in p)
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
        baked = " | 5e-2" if out_of_domain else (
            " | 3e-4" if legacy and kind == "space" else "")
        print(f"{ident} | {rgb} | {want} | 1e-5{baked}")
