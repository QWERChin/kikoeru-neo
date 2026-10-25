# The fallback fonts for ass subtitles: Liberation Sans Regular, which JASSUB
# used by default before, and a subset of Noto Sans CJK SC Regular with the
# kana, JIS X 0208 kanji and GB2312 hanzi. Both come from nixpkgs, so the
# fonts, their licenses and the build all follow the nixpkgs revision.
{ pkgs }:

let
  noto = pkgs.noto-fonts-cjk-sans-static;
  liberation = pkgs.liberation_ttf;
  python = pkgs.python3.withPackages (p: [ p.fonttools p.brotli ]);
in
pkgs.runCommand "kikoeru-neo-fallback-fonts"
{
  nativeBuildInputs = [ python ];
}
  ''
    mkdir -p $out
    python3 -I ${./fallback-fonts.py} latin \
      ${liberation}/share/fonts/truetype/LiberationSans-Regular.ttf $out/fallback-latin.woff2
    python3 -I ${./fallback-fonts.py} cjk \
      ${noto.src}/Sans/OTC/NotoSansCJK-Regular.ttc $out/fallback-cjk.woff2

    # Liberation's LICENSE starts with its copyright notices. noto-cjk keeps
    # its notice in the font's name table, so it is written out here.
    {
      echo "Kikoeru Fallback Latin is Liberation Sans Regular ${liberation.version}, renamed and without hinting."
      echo
      cat ${liberation}/share/doc/liberation-fonts-${liberation.version}/LICENSE
    } > $out/LICENSE-latin.txt
    {
      echo "Kikoeru Fallback CJK is a subset of Noto Sans CJK SC Regular ${noto.version}."
      echo "Copyright 2014-2021 Adobe (http://www.adobe.com/)."
      echo "Noto is a trademark of Google Inc."
      echo
      cat ${noto.src}/LICENSE
    } > $out/LICENSE-cjk.txt
  ''
