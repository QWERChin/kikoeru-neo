# Keep this environment independent of application sources for Docker layer caching.
{ pkgs }:

pkgs.mkShell {
  packages = [
    pkgs.cargo
    pkgs.curl
    pkgs.ffmpeg-headless
    pkgs.mold
    pkgs.rustc
    pkgs.rustfmt
    pkgs.clippy
    pkgs.rust-analyzer
    pkgs.mdbook
    pkgs.nodejs_22
    pkgs.pnpm
    # scripts/bench.py
    pkgs.python3
    pkgs.typescript
    pkgs.sqlx-cli
    # A throwaway server for the PostgreSQL tests (KIKOERU_TEST_PG).
    pkgs.postgresql
  ];
  RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
  # `pnpm e2e` uses these browsers. @playwright/test in web/package.json
  # must have the same version as playwright-driver here.
  PLAYWRIGHT_BROWSERS_PATH = "${pkgs.playwright-driver.browsers}";
  PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS = "true";
  # The interface is Chinese. Without a CJK font the e2e screenshots
  # show empty boxes on hosts that have none installed.
  FONTCONFIG_FILE = pkgs.makeFontsConf { fontDirectories = [ pkgs.noto-fonts-cjk-sans ]; };
  # `pnpm dev` and `pnpm build` take the ass fallback fonts from here.
  KIKOERU_FALLBACK_FONTS_DIR = "${import ./fallback-fonts.nix { inherit pkgs; }}";
}
