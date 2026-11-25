"""Builds assets/fonts/Inter-Rigometry.ttf from the official Inter release.

egui lays out text without OpenType features, so the tabular figures Inter
offers through `tnum` are made the default digits here; numbers then keep their
width while live readings change. Layout tables egui never reads are removed.
The variable weight and optical-size axes are kept.

    python -m pip install fonttools==4.68.0
    python -I scripts/build-font.py path/to/InterVariable.ttf assets/fonts/Inter-Rigometry.ttf

Use InterVariable.ttf from Inter-4.1.zip (https://github.com/rsms/inter/releases/tag/v4.1).
"""

import sys

from fontTools import subset
from fontTools.ttLib import TTFont

source, target = sys.argv[1:3]
font = TTFont(source)

# Tabular digits: point 0-9 at the glyphs the `tnum` feature would select.
gsub = font["GSUB"].table
tabular = {}
for record in gsub.FeatureList.FeatureRecord:
    if record.FeatureTag == "tnum":
        for index in record.Feature.LookupListIndex:
            for table in gsub.LookupList.Lookup[index].SubTable:
                tabular.update(getattr(table, "mapping", {}))
digits = {ord(c) for c in "0123456789"}
for table in font["cmap"].tables:
    if table.isUnicode():
        for code in digits & table.cmap.keys():
            # Format 4 and 12 subtables can share one mapping.
            table.cmap[code] = tabular.get(table.cmap[code], table.cmap[code])

options = subset.Options()
options.unicodes = sorted(font.getBestCmap())
options.layout_features = []
options.drop_tables += ["GSUB", "GPOS", "GDEF", "DSIG"]
options.name_IDs = ["*"]
options.name_languages = ["*"]
options.notdef_outline = True
options.glyph_names = False
options.hinting = False
subsetter = subset.Subsetter(options)
subsetter.populate(unicodes=options.unicodes)
subsetter.subset(font)

advances = {font["hmtx"][font.getBestCmap()[code]][0] for code in digits}
assert len(advances) == 1, f"digits must share one advance, got {advances}"
font.save(target)
