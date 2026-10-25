# Tests for the kikoeru-code-review skill's scripts. They run against a fake
# Codex and a fake ps, so no model is called.
{ pkgs }:

pkgs.runCommand "kikoeru-code-review-check"
  {
    nativeBuildInputs = [ pkgs.bats pkgs.shellcheck pkgs.git pkgs.python3 pkgs.util-linux pkgs.procps ];
  }
  ''
    cp -r ${../.agents/skills/kikoeru-code-review} skill
    chmod -R u+w skill
    # The sandbox has no /usr/bin/env for the scripts' shebangs.
    patchShebangs skill
    shellcheck skill/scripts/*.sh skill/tests/fake-*
    bats skill/tests/codex-review.bats
    touch $out
  ''
