"""Generate tests/fixtures/grading.fixture with the reference OpenColorIO library.

Every colour space in config.ocio beside it carries exactly one
GradingPrimaryTransform or GradingToneTransform on from_reference, so each space
gives two rows per probe: `space: ref -> <name>` runs that transform forwards and
`space: <name> -> ref` runs the inverse the reference library defines for it.

Run it from tests/fixtures/grading:

    python generate.py > ../grading.fixture

The probe set is the one tests/fixtures/generate.py already uses, read from that
file rather than copied, so the two reference fixtures ask the same questions.
"""
import ast
import datetime
import pathlib
import re

import PyOpenColorIO as ocio

HERE = pathlib.Path(__file__).resolve().parent
source = (HERE.parent / "generate.py").read_text()
probes = ast.literal_eval(re.search(r"probes = (\[.*?\])\n", source, re.S).group(1))

cfg = ocio.Config.CreateFromFile(str(HERE / "config.ocio"))
REFERENCE = "ref"

# The bounds, each a measured fact rather than a number chosen to make a row
# pass, and each written to be tightened and never widened.
#
# EXACT is the promise, and it is what all but two families of row carry: the
# grading maths is arithmetic, and this crate's port of it agrees with the
# reference to about a part in a hundred million.
#
# The other two are places where the REFERENCE's answer is the approximate one.
# Its x86 build raises to a power and takes a logarithm with polynomials
# (ssePower and sseLog2 in SSE.h) where this crate calls the real functions, and
# the two then part company:
#
#   SSE_POW  the log and video styles' gamma, and the linear style's contrast.
#            Measured 1.5e-5 over this probe set. The linear style run backwards
#            carries it whatever its contrast says, because the reference's SSE
#            branch raises to the power even where its own scalar branch skips
#            the step as an identity: that is the reference disagreeing with
#            itself, and this crate follows the scalar reading.
#   SSE_LOG  the tone grade's linear style, which goes to a log view of the
#            light and back around the five bands. A near-identity tone grade
#            run through that round trip alone measures 7.1e-5 at a probe of 64,
#            so the bands are not what costs it. Measured 9.3e-5.
#
# Neither can be tightened by trying harder, and the pre-render skips the power
# step outright when it would be the identity, which is why a grade that only
# moves brightness or offset still carries EXACT.
EXACT, SSE_POW, SSE_LOG = "1e-5", "5e-5", "2e-4"

# The baked gate's bounds, and only for the rows that need one: a chain whose
# steps each keep the channels apart bakes to a 16385-sample curve, and that
# curve costs nothing measurable on top of the row's own bound, so those rows
# take the artefact's own default (docs/impl/ocio.md 5.4). A saturation mixes
# the channels, so a primary grade with one bakes to a 65-cube instead.
#
#   CUBE     an ordinary cube row. Measured 4.9e-3.
#   CORNER   a cube row on a space that states a clamp. A clamp is a kink, the
#            eight cube samples around it straddle it, and no interpolation
#            between them lands on it. Measured 2.1e-2, at a probe sitting
#            exactly on the stated clamp.
CUBE, CORNER = "5e-3", "3e-2"

NO_CLAMP_BLACK, NO_CLAMP_WHITE = -1.7976931348623157e308, 1.7976931348623157e308


def flat(rgbm):
    """Whether an rgb-plus-master value is the identity in all four slots."""
    return (rgbm.red, rgbm.green, rgbm.blue, rgbm.master) == (1.0, 1.0, 1.0, 1.0)


def bounds(name, flipped):
    """The two bounds a row carries, the second empty where none is due.

    `flipped` says the row reads the space back to the reference, so the
    transform runs the opposite way to the direction it states.
    """
    transform = cfg.getColorSpace(name).getTransform(ocio.COLORSPACE_DIR_FROM_REFERENCE)
    style = transform.getStyle()
    stated = transform.getDirection() == ocio.TRANSFORM_DIR_INVERSE
    inverse = stated != flipped
    if isinstance(transform, ocio.GradingToneTransform):
        return (SSE_LOG if style == ocio.GRADING_LIN else EXACT), None
    value = transform.getValue()
    linear = style == ocio.GRADING_LIN
    powered = not flat(value.contrast if linear else value.gamma)
    if linear and inverse and value.saturation in (0.0, 1.0):
        powered = True
    if value.saturation == 1.0:
        return (SSE_POW if powered else EXACT), None
    clamped = (
        value.clampBlack != NO_CLAMP_BLACK or value.clampWhite != NO_CLAMP_WHITE
    )
    return (SSE_POW if powered else EXACT), (CORNER if clamped else CUBE)


print("# grading: expected values from the REFERENCE OpenColorIO library.")
print(f"# library: PyOpenColorIO {ocio.GetVersion()}")
print("# config: grading/config.ocio, one grading transform per colour space;")
print("#   the first ten sets are verbatim from Blender 5.2 and PixelManager.")
print(f"# generated: {datetime.date.today()} by tests/fixtures/grading/generate.py")
print("# A fifth field is the BAKED gate's own bound, for the rows that bake to")
print("#   a cube; the rest take the artefact's own default.")

for name in cfg.getColorSpaceNames():
    if name == REFERENCE:
        continue
    for src, dst in ((REFERENCE, name), (name, REFERENCE)):
        exact, baked = bounds(name, src == name)
        tail = f" | {baked}" if baked else ""
        cpu = cfg.getProcessor(src, dst).getDefaultCPUProcessor()
        for probe in probes:
            rgb = " ".join(f"{v:.8f}" for v in probe)
            want = " ".join(f"{v:.8f}" for v in cpu.applyRGB(list(probe)))
            print(f"space: {src} -> {dst} | {rgb} | {want} | {exact}{tail}")
