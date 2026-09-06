"""Generate the look-up table FILE fixture with the reference OpenColorIO library.

    python files-generate.py > files.fixture

The companion to generate.py: that one tabulates whole config edges, this one
tabulates one look-up table file at a time, so a reader fault shows up as the
file it is in rather than as a tolerance somewhere in a config.

Five files sit in luts/. Two are real, taken from the configs that named them:
xyz_E_to_D65.spimtx is in both Blender's bundled config and PixelManager, and
the FilmLight T-Log .cub is PixelManager's smallest Truelight file and carries
an input LUT with no cube after it. The other three are written here, because
no small real file covers the paths that go quietly wrong: a matrix with a
non-zero offset column (the divide by 65535), a Truelight cube whose input LUT
counts in cube cells (the divide by width-1), and a .3dl with a shaper that
bends and a cube written blue fastest (the transposition).

Rerunning this rewrites those three byte for byte and prints the table.
"""
import datetime
import os
import sys

import PyOpenColorIO as ocio

# The table is checked in with Unix line endings like every other fixture, so
# the redirect writes them whatever platform this runs on.
sys.stdout.reconfigure(newline="\n")

HERE = os.path.dirname(os.path.abspath(__file__))
LUTS = os.path.join(HERE, "luts")

# The probe set generate.py uses, unchanged: neutrals, primaries, denormals,
# negatives, and values far past one. Every file here is a table over 0 to 1,
# so the last few gate the clamp at the ends as well as the maths inside.
probes = [
    [0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [0.18, 0.18, 0.18], [0.5, 0.25, 0.75],
    [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 1.0, 0.0],
    [0.004, 0.002, 0.001], [1e-7, 1e-7, 1e-7],
    [-0.05, 0.4, 0.9], [-0.2, -0.1, 0.02],
    [2.0, 1.4, 0.3], [16.0, 8.0, 4.0], [100.0, 12.0, -0.3], [64.0, 64.0, 64.0],
]

# A config with somewhere to bake to and luts/ on its search path. The grade is
# different on all three channels on purpose: a cube read in the wrong order
# still passes every neutral, and only an asymmetric one catches it.
CONFIG = """ocio_profile_version: 2
search_path: luts
roles:
  reference: lin
  scene_linear: lin
  default: lin
displays:
  none:
    - !<View> {name: n, colorspace: lin}
colorspaces:
  - !<ColorSpace>
    name: lin
  - !<ColorSpace>
    name: graded
    from_scene_reference: !<CDLTransform> {slope: [1.1, 0.9, 1.0], offset: [0.02, 0.0, 0.05], power: [1.0, 1.2, 0.8]}
"""
cfg = ocio.Config.CreateFromStream(CONFIG)
graded = cfg.getProcessor("lin", "graded").getDefaultCPUProcessor()


def write_offset_spimtx(path):
    """A 3x4 matrix whose fourth column is not zero.

    Neither real .spimtx has an offset, so neither would notice the divide by
    65535 going missing. 6553.5 is a tenth of full scale, so the offset this
    file asks for is 0.1.
    """
    with open(path, "w", newline="\n") as f:
        f.write("0.90 0.05 0.05 6553.5\n"
                "0.05 0.90 0.05 -3276.75\n"
                "0.05 0.05 0.90 0\n")


def write_baked_cub(path):
    """A Truelight cube from the reference library's own writer.

    Its input LUT runs 0 to width-1 rather than 0 to 1, which is the whole
    reason this file is here: a reader that skips the divide sends every value
    off the top of the cube.
    """
    baker = ocio.Baker()
    baker.setConfig(cfg)
    baker.setFormat("truelight")
    baker.setInputSpace("lin")
    baker.setTargetSpace("graded")
    baker.setCubeSize(8)
    baker.setShaperSize(16)
    with open(path, "w", newline="\n") as f:
        f.write(baker.bake())


def write_3dl(path, size=8, shaper_size=33):
    """A Flame table: a shaper that bends, then a cube written blue fastest.

    The reference library's own .3dl writer always writes a straight shaper,
    which the reader then throws away, so a baked file would leave the shaper
    path untested. This writes one that bends. The numbers are code values, the
    shaper at 10 bits and the cube at 12, which is what the reader has to infer
    from the largest number in each.
    """
    lines = []
    shaper = [round(1023.0 * (i / (shaper_size - 1)) ** (1.0 / 1.5))
              for i in range(shaper_size)]
    lines.append(" ".join(str(v) for v in shaper))
    for r in range(size):
        for g in range(size):
            for b in range(size):
                rgb = [r / (size - 1), g / (size - 1), b / (size - 1)]
                out = graded.applyRGB(rgb)
                lines.append(" ".join(
                    str(round(min(max(v, 0.0), 1.0) * 4095)) for v in out))
    with open(path, "w", newline="\n") as f:
        f.write("\n".join(lines) + "\n")


write_offset_spimtx(os.path.join(LUTS, "offset.spimtx"))
write_baked_cub(os.path.join(LUTS, "truelight_shaper_cube.cub"))
write_3dl(os.path.join(LUTS, "flame_shaper_cube.3dl"))

# Real configs write `interpolation: tetrahedral` on every file that carries a
# cube and `linear` on the rest, which is what Lumit's two samplers are. Setting
# it here is agreement with the configs, not a choice.
#
# The third column says whether the reference can run the file backwards. It can
# invert a matrix and a curve; a cube it can only approximate, which Lumit
# refuses outright, so those files are tabulated forwards only.
#
# The fourth column says what the file holds, and the two bounds below are
# about the BAKED gate alone: read exactly, the chain IS the file, and every
# row here holds 1e-5.
#
# CURVE is README.md's own measured number, unchanged. Lumit resamples a file
# curve through its own shaper table where the reference reads the file
# directly, and that second interpolation measures 8.9e-5 on the legacy ACES
# config and 8.6e-5 here.
#
# TOP is 5.4's stated domain edge, met at a FILE's edge. A table stops at 1.0
# and clamps above it, and the baked form has no sample sitting exactly on that
# corner, so the cell containing it blends the last real value with the clamp.
# T-Log measures 7.6e-3 relative there (its own last cell jumps 125.9375 to
# 128.0, twice the step before it) and the two cube files 3.3e-2, which is the
# same 5e-2 generate.py already writes for a probe past the shaper's reach and
# for the same reason: the shaper spends its grid on 0 to 32, so its samples
# either side of 1.0 are far apart. Making the cube in the file finer does not
# move it, which is what says it is the bake's grid and not the file's.
CURVE, TOP = "3e-4", "5e-2"

MATRIX, CURVE_FILE, CUBE_FILE = 0, 1, 2

FILES = [
    ("xyz_E_to_D65.spimtx", ocio.INTERP_LINEAR, True, MATRIX),
    ("offset.spimtx", ocio.INTERP_LINEAR, True, MATRIX),
    ("FilmLight_TLog_EGamut2_2_FilmLight_Linear_EGamut2.cub", ocio.INTERP_LINEAR, True, CURVE_FILE),
    ("truelight_shaper_cube.cub", ocio.INTERP_TETRAHEDRAL, False, CUBE_FILE),
    ("flame_shaper_cube.3dl", ocio.INTERP_TETRAHEDRAL, False, CUBE_FILE),
]

print("# Look-up table files: expected values from the REFERENCE OpenColorIO library.")
print(f"# library: PyOpenColorIO {ocio.GetVersion()}")
print("# files: tests/fixtures/luts/, two real and three written by this script.")
print("#   xyz_E_to_D65.spimtx is Blender 5.2's and PixelManager's own copy;")
print("#   the FilmLight .cub is PixelManager/TCAMv3, an input LUT with no cube.")
print(f"# generated: {datetime.date.today()} by tests/fixtures/files-generate.py")
print("# A row reads `file: <path>` forwards, `file-inverse: <path>` backwards.")
print("# A fifth field is the BAKED gate's own bound, written where the row sits")
print("#   on the top of a file table's domain, or where Lumit resamples a file")
print("#   curve through its own shaper.")

for name, interp, invertible, holds in FILES:
    for prefix, direction in (("file", ocio.TRANSFORM_DIR_FORWARD),
                              ("file-inverse", ocio.TRANSFORM_DIR_INVERSE)):
        if direction == ocio.TRANSFORM_DIR_INVERSE and not invertible:
            continue
        transform = ocio.FileTransform(src=name, interpolation=interp)
        cpu = cfg.getProcessor(transform, direction).getDefaultCPUProcessor()
        for p in probes:
            out = cpu.applyRGB(list(p))
            rgb = " ".join(f"{v:.8f}" for v in p)
            want = " ".join(f"{v:.8f}" for v in out)
            forward = direction == ocio.TRANSFORM_DIR_FORWARD
            if holds != MATRIX and forward and any(v == 1.0 for v in p):
                tail = f" | {TOP}"
            elif holds == CURVE_FILE:
                tail = f" | {CURVE}"
            else:
                tail = ""
            print(f"{prefix}: luts/{name} | {rgb} | {want} | 1e-5{tail}")
