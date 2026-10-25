"""Print how many Codex reviews of one repository are running.

    count_reviews.py <proc-dir> <git-common-dir> <worktree>...

A process counts when its command is `codex` or `codex-raw` and it reviews
this repository: either the launcher started it for this repository (its
environment has KCR_REVIEW_RUNNING=<git-common-dir>), or it is a review
started by hand (`codex review` or `codex exec review`) whose working
directory, or `-C` directory, is inside one of the repository's worktrees.
Codex working on other projects does not count, and neither does a plain
`codex exec`, which is ordinary work, often an agent that is itself waiting
for a review.

Command lines come from /proc/<pid>/cmdline, which keeps argument
boundaries, so a path with spaces cannot hide the subcommand. A wrapper such
as `timeout ... codex exec` is not counted, so each review counts once.
"""

import os
import sys

# Options that take the next argument as their value, before or after the
# subcommand.
VALUE_OPTIONS = {
    "-c", "--config", "-m", "--model", "-p", "--profile", "-C", "--cd",
    "-s", "--sandbox", "-a", "--ask-for-approval", "-i", "--image",
    "--enable", "--disable", "--add-dir", "--local-provider",
    "-o", "--output-last-message", "--output-schema", "--color",
}


def positionals(args):
    args = iter(args)
    for arg in args:
        if arg in VALUE_OPTIONS:
            next(args, None)
        elif not arg.startswith("-"):
            yield arg


def is_codex(argv):
    return bool(argv) and os.path.basename(argv[0]) in ("codex", "codex-raw")


def is_manual_review(argv):
    words = positionals(argv[1:])
    subcommand = next(words, None)
    if subcommand == "review":
        return True
    return subcommand in ("exec", "e") and next(words, None) == "review"


def working_dir(argv, proc_cwd):
    # -C or --cd moves Codex to another directory, relative to its cwd.
    for flag, value in zip(argv, argv[1:]):
        if flag in ("-C", "--cd"):
            return os.path.realpath(os.path.join(proc_cwd, value))
    return proc_cwd


def inside(path, roots):
    return any(path == root or path.startswith(root + os.sep) for root in roots)


def read_nul_list(path):
    try:
        with open(path, "rb") as f:
            raw = f.read()
    except OSError:
        return []
    if raw.endswith(b"\0"):
        raw = raw[:-1]
    return raw.split(b"\0") if raw else []


def main():
    if len(sys.argv) < 3:
        print(__doc__, file=sys.stderr)
        return 2
    proc, common = sys.argv[1], sys.argv[2]
    roots = [os.path.realpath(root) for root in sys.argv[3:]]
    marker = os.fsencode("KCR_REVIEW_RUNNING=" + common)
    count = 0
    for pid in os.listdir(proc):
        if not pid.isdigit():
            continue
        base = os.path.join(proc, pid)
        argv = [arg.decode(errors="replace") for arg in read_nul_list(os.path.join(base, "cmdline"))]
        # The rest is read only for Codex, since this runs every tenth of a
        # second while a review is being admitted.
        if not is_codex(argv):
            continue
        if marker in read_nul_list(os.path.join(base, "environ")):
            count += 1
        elif is_manual_review(argv):
            try:
                cwd = os.path.realpath(os.readlink(os.path.join(base, "cwd")))
            except OSError:
                continue
            if inside(working_dir(argv, cwd), roots):
                count += 1
    print(count)
    return 0


if __name__ == "__main__":
    sys.exit(main())
