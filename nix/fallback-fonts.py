# Builds the two fallback fonts JASSUB uses for ass subtitles when a work
# ships no font for a glyph (book: frontend.md, 回退字体).
#
# usage: fallback-fonts.py latin <LiberationSans-Regular.ttf> <out.woff2>
#        fallback-fonts.py cjk <NotoSansCJK-Regular.ttc> <out.woff2>
#
# Both get the family name FAMILY. libass knows one default family only and
# takes a missing glyph from the first font of that family that has it, so
# sharing the name lets Liberation Sans keep its Latin, Greek and Cyrillic
# and the CJK subset fill in the rest.
import sys

from fontTools import subset
from fontTools.ttLib import TTFont

FAMILY = "Kikoeru Fallback"

# The SC face of the Noto Sans CJK OTC. Its default glyphs follow the
# Simplified Chinese forms, which matches the interface language.
SC_FACE = 2

# Whole blocks, small next to the ideographs: ASCII, Latin-1, general
# punctuation, CJK punctuation, kana and the full-width forms.
CJK_BLOCKS = [
    (0x0020, 0x007E),
    (0x00A0, 0x00FF),
    (0x2000, 0x206F),
    (0x3000, 0x303F),
    (0x3040, 0x309F),
    (0x30A0, 0x30FF),
    (0x31F0, 0x31FF),
    (0xFF00, 0xFFEF),
]


def double_byte(codec: str, lead: range, trail: range) -> set[str]:
    """Every character a two-byte EUC code set can encode."""
    chars: set[str] = set()
    for a in lead:
        for b in trail:
            try:
                chars.update(bytes([a, b]).decode(codec))
            except UnicodeDecodeError:
                pass
    return chars


def cjk_charset() -> set[int]:
    # JIS X 0208 (levels 1 and 2 plus its symbols) and GB2312 (all 6763
    # hanzi plus its symbols), enumerated from Python's codecs instead of a
    # committed table.
    chars = double_byte("euc_jp", range(0xA1, 0xFF), range(0xA1, 0xFF))
    chars |= double_byte("gb2312", range(0xA1, 0xF8), range(0xA1, 0xFF))
    points = {ord(c) for c in chars}
    for first, last in CJK_BLOCKS:
        points.update(range(first, last + 1))
    return points


def rename(font: TTFont, part: str) -> None:
    # Both are modified versions under the OFL, and Liberation is a Reserved
    # Font Name. A name of their own also keeps libass from taking either
    # for a real Liberation Sans or Noto Sans CJK that a work ships.
    full = f"{FAMILY} {part}"
    postscript = f"KikoeruFallback-{part}"
    names = {1: FAMILY, 2: "Regular", 3: f"{postscript};kikoeru-neo", 4: full, 6: postscript}
    table = font["name"]
    for name_id in (*names, 16, 17, 18, 21, 22):
        table.removeNames(nameID=name_id)
    for name_id, value in names.items():
        table.setName(value, name_id, 3, 1, 0x409)
    if "CFF " in font:
        cff = font["CFF "].cff
        cff.fontNames = [postscript]
        top = cff.topDictIndex[0]
        top.FullName = full
        top.FamilyName = FAMILY


def build(font: TTFont, unicodes: set[int] | None, drop_locl: bool) -> None:
    options = subset.Options()
    # Keep the copyright, trademark and license records.
    options.name_IDs = ["*"]
    options.name_languages = ["*"]
    # libass renders without hinting, and flat charstrings compress better.
    options.hinting = False
    options.desubroutinize = True
    options.notdef_outline = True
    if drop_locl:
        # locl would pull in the Japanese, Korean and Traditional variants
        # of every kept ideograph and nearly double the size.
        options.layout_features = [f for f in options.layout_features if f != "locl"]
    subsetter = subset.Subsetter(options)
    subsetter.populate(unicodes=unicodes if unicodes is not None else font.getBestCmap().keys())
    subsetter.subset(font)


def main(kind: str, source: str, target: str) -> None:
    # The source timestamp is kept so that two builds give the same bytes.
    if kind == "latin":
        font = TTFont(source, recalcTimestamp=False)
        build(font, None, drop_locl=False)
        rename(font, "Latin")
    elif kind == "cjk":
        font = TTFont(source, fontNumber=SC_FACE, recalcTimestamp=False)
        build(font, cjk_charset(), drop_locl=True)
        rename(font, "CJK")
    else:
        raise SystemExit(f"unknown font kind {kind!r}")
    font.flavor = "woff2"
    font.save(target)


if __name__ == "__main__":
    main(*sys.argv[1:])
