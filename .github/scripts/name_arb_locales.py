# Rewrite every translated .arb's "@@locale" to the locale its filename says.
#
# Run by .github/workflows/translation-locale.yml on the branch Crowdin syncs
# to; see that file for why this is needed. The source file (app_en.arb) is a
# human's and is left alone.
#
# A plain text substitution rather than a JSON round trip: re-serialising would
# reorder and reformat a translator's file for the sake of one word.

import pathlib
import re

L10N = pathlib.Path("flutter_ui/lib/l10n")

for arb in sorted(L10N.glob("app_*.arb")):
    if arb.name == "app_en.arb":
        continue
    want = arb.stem[len("app_") :]
    text = arb.read_text(encoding="utf-8")
    fixed, count = re.subn(
        r'("@@locale"\s*:\s*)"[^"]*"', r'\1"%s"' % want, text, count=1
    )
    if not count:
        # No key at all is legal — Flutter reads the locale from the filename —
        # so there is nothing to disagree with and nothing to mend.
        continue
    if fixed != text:
        arb.write_text(fixed, encoding="utf-8", newline="")
        print(f"{arb.name}: @@locale -> {want}")
