"""Bake Film's stock data from the spektrafilm profiles.

The film stocks are Andrea Volpato's spektrafilm profiles
(https://github.com/andreavolpato/spektrafilm, CC BY-SA 4.0). This reads them
from a spektrafilm checkout and writes `stocks.json` beside this file, cut down
to what Film reads: each stock's dye spectra, base, curves, coupler matrix,
grain table and, for the stocks a camera loads, its exposure matrix. The
spectral maths that follows from them runs in `tables.rs`.

Needs numpy, scipy and colour-science. Run from this directory:

    python bake.py <path to spektrafilm>/src/spektrafilm > stocks.json
"""
import json, sys
import numpy as np
import scipy.interpolate
import colour

ROOT = sys.argv[1]
SHAPE = colour.SpectralShape(380, 780, 5)
WAVES = SHAPE.wavelengths
LOG_E = np.linspace(-3, 4, 256)

# Film's list, in its order. Prints last.
FILMING = [
    "kodak_vision3_50d", "kodak_vision3_250d", "kodak_vision3_200t", "kodak_vision3_500t",
    "kodak_portra_160", "kodak_portra_400", "kodak_portra_800", "kodak_ektar_100",
    "kodak_gold_200", "kodak_ultramax_400", "fujifilm_c200", "fujifilm_pro_400h",
    "fujifilm_xtra_400", "kodak_ektachrome_100", "kodak_kodachrome_64",
    "fujifilm_provia_100f", "fujifilm_velvia_100",
]
PRINTS = ["kodak_2383", "kodak_2393", "kodak_portra_endura", "fujifilm_crystal_archive_typeii"]

# spektrafilm's grain defaults (runtime/params_schema.py GrainParams).
PARTICLE_AREA = 0.2
SCALE_RGB = np.array([1.6, 1.6, 3.2])
SCALE_LAYERS = np.array([2.0, 1.0, 0.5])
UNIFORMITY = np.array([0.97, 0.99, 0.97])
DENSITY_MIN = 0.03


def illuminant(name):
    """spektrafilm's model/illuminants.py, normalised to a mean of 1."""
    if name == "T":
        sd = colour.SDS_LIGHT_SOURCES["Incandescent"].copy().align(SHAPE)
    elif name == "K75P":
        sd = colour.SDS_LIGHT_SOURCES["Kinoton 75P"].copy().align(SHAPE)
    elif name == "TH-KG3":
        sd = colour.sd_blackbody(3400, SHAPE)
        kg3 = np.loadtxt(f"{ROOT}/data/filters/heat_absorbing/schott/KG3.csv", delimiter=",")
        kg3 = kg3[np.unique(kg3[:, 0], return_index=True)[1]]
        sd.values = sd.values * scipy.interpolate.Akima1DInterpolator(kg3[:, 0], kg3[:, 1])(WAVES)
    else:
        sd = colour.SDS_ILLUMINANTS[name].copy().align(SHAPE)
    v = np.nan_to_num(np.asarray(sd.values, dtype=float))
    return v / v.mean()


def fill(a):
    """NaNs at the ends of a measured curve take the nearest real value."""
    a = np.array(a, dtype=float)
    for c in range(a.shape[1]):
        ok = ~np.isnan(a[:, c])
        a[:, c] = np.interp(np.arange(len(a)), np.flatnonzero(ok), a[ok, c])
    return a


def couplers(stock, positive):
    """spektrafilm's runtime/params_builder.py: donor row, receiver column."""
    if stock == "fujifilm_velvia_100":
        s, rg, gb, br = (0.108, 0.072, 0.054), (0.108, 0.054), (0.072, 0.054), (0.054, 0.054)
    elif stock == "fujifilm_provia_100f":
        s, rg, gb, br = (0.156, 0.104, 0.078), (0.156, 0.078), (0.104, 0.078), (0.078, 0.078)
    elif positive:
        s, rg, gb, br = (0.12, 0.08, 0.06), (0.12, 0.06), (0.08, 0.06), (0.06, 0.06)
    else:
        s, rg, gb, br = (0.336, 0.319, 0.273), (0.353, 0.302), (0.154, 0.353), (0.168, 0.226)
    return [[s[0], rg[0], rg[1]], [gb[0], s[1], gb[1]], [br[0], br[1], s[2]]]


def grain_table(curves, layers, positive):
    """Grain variance by channel density, as density² × µm².

    spektrafilm's model/grain.py draws each sub-layer's developed grains as a
    Poisson count thinned binomially. Its variance has a closed form,
    Dmax² × (1 - p u) × p ÷ n, and n is per pixel area, so the sum over the
    three sub-layers times the pixel's area is a fixed function of density.
    """
    dmax_l = np.nanmax(layers, axis=0)  # [sub-layer, channel]
    frac = dmax_l / dmax_l.sum(axis=0)
    out = np.zeros((64, 3))
    for c in range(3):
        ds = np.linspace(0, curves[:, c].max(), 64)
        for l in range(3):
            if positive:
                d_l = np.interp(-ds, -curves[:, c], layers[:, l, c])
            else:
                d_l = np.interp(ds, curves[:, c], layers[:, l, c])
            lo = frac[l, c] * DENSITY_MIN
            top = dmax_l[l, c] + lo
            p = np.clip((d_l + lo) / top, 1e-6, 1 - 1e-6)
            area = PARTICLE_AREA * SCALE_RGB[c] * SCALE_LAYERS[l]
            out[:, c] += top * top * (1 - p * UNIFORMITY[c]) * p * area / frac[l, c]
    return out


def to_rec709(view):
    """XYZ under the viewing light to linear Rec. 709, adapted to D65."""
    cmf = colour.MSDS_CMFS["CIE 1931 2 Degree Standard Observer"].copy().align(SHAPE).values
    xyz = view @ cmf / (view @ cmf[:, 1])
    m = colour.XYZ_to_RGB(np.eye(3), "sRGB", illuminant=colour.XYZ_to_xy(xyz),
                          chromatic_adaptation_transform="CAT02")
    return m.T


def r(a, n=5):
    return np.vectorize(lambda v: float(f"{v:.{n}g}"))(np.asarray(a, dtype=float)).tolist()


def main():
    cmf = colour.MSDS_CMFS["CIE 1931 2 Degree Standard Observer"].copy().align(SHAPE).values
    basis = colour.recovery.MSDS_BASIS_FUNCTIONS_sRGB_MALLETT2019.copy().align(SHAPE).values
    stocks = []
    for name in FILMING + PRINTS:
        p = json.load(open(f"{ROOT}/data/profiles/{name}.json"))
        info, data = p["info"], p["data"]
        positive = info["type"] == "positive"
        printing = info["stage"] == "printing"
        sens = np.nan_to_num(10 ** np.array(data["log_sensitivity"], dtype=float))
        entry = {
            "id": name,
            "name": info["name"],
            "kind": "print" if printing else ("reversal" if positive else "negative"),
            "use": info["use"],
            "backing": info["antihalation"],
            "dye": r(fill(data["channel_density"])),
            "base": r(np.nan_to_num(np.array(data["base_density"], dtype=float))),
            "view": info["viewing_illuminant"],
        }
        if printing:
            # Prints develop through the fitted model (develop_print_morph).
            m = data["density_curves_model"]
            mu, amp, sig = (np.array(m[k]) for k in ("centers", "amplitudes", "sigmas"))
            from scipy.stats import norm
            curves = np.stack([
                sum(amp[c, i] * norm.cdf((LOG_E - mu[c, i]) / sig[c, i]) for i in range(3))
                for c in range(3)], axis=1)
            entry["curves"] = r(curves)
            entry["sens"] = r(sens)
        else:
            curves = fill(data["density_curves"])
            curves -= curves.min(axis=0)
            entry["curves"] = r(curves)
            entry["couplers"] = couplers(name, positive)
            layers = np.array(data["density_curves_layers"], dtype=float)
            entry["grain"] = r(grain_table(curves, np.nan_to_num(layers), positive))
            # Camera exposure: Mallett and Yuksel's basis lit by the stock's
            # reference light, read by its layers, one stop of grey to one.
            light = illuminant(info["reference_illuminant"])
            cam = (basis * light[:, None]).T @ sens  # [basis, layer]
            grey = 0.184 * light @ sens
            entry["camera"] = r((cam / grey[None, :]).T)
        stocks.append(entry)
    out = {
        "about": "Film stocks from spektrafilm by Andrea Volpato, "
                 "https://github.com/andreavolpato/spektrafilm, CC BY-SA 4.0 "
                 "(https://creativecommons.org/licenses/by-sa/4.0/). Changed: cut down to "
                 "the arrays Film reads, grain reduced to its variance, camera exposure "
                 "reduced to a matrix, by bake.py. This file stays CC BY-SA 4.0.",
        "cmf": r(cmf),
        "enlarger": r(illuminant("TH-KG3")),
        "views": {v: {"light": r(illuminant(v)), "to_rec709": r(to_rec709(illuminant(v)), 7)}
                  for v in ("K75P", "D50")},
        "stocks": stocks,
    }
    json.dump(out, sys.stdout, separators=(",", ":"))


main()
