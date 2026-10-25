"""Render kikoeru-code-review JSON as Markdown.

    render_review.py single <review.json> <output.md> [key=value ...]
    render_review.py merge <output.md> <review.json>...

`single` renders one review. The key=value pairs (mode, target, head, model,
effort, tier) go into the header. `merge` ranks the findings of several
reviews, such as one per component, in one report, names each finding's
source review after its JSON file, and shows the commit each review saw
from the `.head` file the launcher writes next to the JSON.
"""

import json
import os
import sys
import tempfile
from pathlib import Path

SEVERITIES = ["P0", "P1", "P2", "P3"]
VERDICT_ZH = {"confirmed": "已核实", "uncertain": "未完全核实"}


def load(path):
    data = json.loads(Path(path).read_text(encoding="utf-8"))
    for key in ("summary", "coverage_notes", "findings", "retracted"):
        if key not in data:
            raise SystemExit(f"render_review.py: {path} has no {key!r}")
    return data


def ranked(findings):
    # Confirmed findings come before uncertain ones of the same severity.
    return sorted(
        findings,
        key=lambda f: (SEVERITIES.index(f["severity"]), f["verdict"] != "confirmed"),
    )


def counts_line(findings):
    counts = [f"{s} {sum(f['severity'] == s for f in findings)}" for s in SEVERITIES]
    return "，".join(counts)


def finding_block(n, f, source=None):
    tag = f"[{source}] " if source else ""
    lines = [
        f"### {n}. {tag}{f['title']}",
        "",
        f"`{f['file']}:{f['line']}`，{f['severity']}，角度 `{f['lens']}`，{VERDICT_ZH[f['verdict']]}。",
    ]
    if f["related_locations"]:
        lines.append("相关位置：" + "、".join(f"`{loc}`" for loc in f["related_locations"]) + "。")
    lines += [
        "",
        f["description"],
        "",
        f"**触发场景**：{f['trigger_scenario']}",
        "",
        f"**证据**：{f['evidence']}",
        "",
        f"**修复方向**：{f['suggested_fix']}",
        "",
        f"**建议测试**：{f['suggested_test']}",
        "",
    ]
    return lines


def retracted_block(items, source=None):
    if not items:
        return []
    lines = ["## 核实时撤回的发现", ""]
    for item in items:
        tag = f"[{source(item)}] " if source else ""
        lines.append(f"- {tag}`{item['file']}:{item['line']}` {item['title']}：{item['reason']}")
    lines.append("")
    return lines


def render_single(data, meta):
    findings = ranked(data["findings"])
    out = ["# kikoeru-code-review", ""]
    target = " ".join(x for x in (meta.get("mode", "?"), meta.get("target", "")) if x)
    out.append(
        f"审查对象 `{target}`，HEAD `{meta.get('head', '?')}`，"
        f"模型 `{meta.get('model', '?')}`，effort `{meta.get('effort', '?')}`，"
        f"tier `{meta.get('tier', '?')}`。"
    )
    out += ["", f"发现：{counts_line(findings)}。", "", "## 总结", "", data["summary"], ""]
    out += [f"覆盖说明：{data['coverage_notes']}", ""]
    if findings:
        out += ["## 发现", ""]
        for n, f in enumerate(findings, 1):
            out += finding_block(n, f)
    out += retracted_block(data["retracted"])
    return "\n".join(out)


def reviewed_head(json_path):
    head = Path(json_path).with_suffix(".head")
    return head.read_text(encoding="utf-8").strip()[:9] if head.exists() else "?"


def render_merge(reviews, heads):
    rows = [(name, f) for name, data in reviews for f in data["findings"]]
    rows.sort(key=lambda r: (SEVERITIES.index(r[1]["severity"]), r[1]["verdict"] != "confirmed", r[0]))
    findings = [f for _, f in rows]
    out = ["# kikoeru-code-review 汇总", "", f"共 {len(reviews)} 份审查，发现：{counts_line(findings)}。", ""]
    out += ["| 审查 | HEAD | " + " | ".join(SEVERITIES) + " |", "|---|---|" + "---|" * len(SEVERITIES)]
    for name, data in reviews:
        cells = [str(sum(f["severity"] == s for f in data["findings"])) for s in SEVERITIES]
        out.append(f"| `{name}` | `{heads[name]}` | " + " | ".join(cells) + " |")
    out.append("")
    for severity in SEVERITIES:
        group = [(i, name, f) for i, (name, f) in enumerate(rows, 1) if f["severity"] == severity]
        if not group:
            continue
        out += [f"## {severity}", ""]
        for i, name, f in group:
            out += finding_block(i, f, source=name)
    retracted = [(name, item) for name, data in reviews for item in data["retracted"]]
    if retracted:
        out += ["## 核实时撤回的发现", ""]
        for name, item in retracted:
            out.append(f"- [{name}] `{item['file']}:{item['line']}` {item['title']}：{item['reason']}")
        out.append("")
    out += ["## 各审查的总结", ""]
    for name, data in reviews:
        out += [f"### `{name}`", "", data["summary"], "", f"覆盖说明：{data['coverage_notes']}", ""]
    return "\n".join(out)


def write_atomically(path, text):
    # A reader, such as a rerun of review_all.sh, sees the old file or the
    # whole new one, never a half-written one.
    path = Path(path)
    fd, tmp = tempfile.mkstemp(dir=path.parent, prefix=f".{path.name}.")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(text)
        os.replace(tmp, path)
    except BaseException:
        os.unlink(tmp)
        raise


def main(argv):
    if len(argv) >= 3 and argv[0] == "single":
        data = load(argv[1])
        meta = dict(arg.split("=", 1) for arg in argv[3:])
        write_atomically(argv[2], render_single(data, meta))
        print(f"kikoeru-code-review: {counts_line(data['findings'])} -> {argv[2]}")
    elif len(argv) >= 3 and argv[0] == "merge":
        reviews = [(Path(p).stem, load(p)) for p in argv[2:]]
        heads = {Path(p).stem: reviewed_head(p) for p in argv[2:]}
        write_atomically(argv[1], render_merge(reviews, heads))
        total = [f for _, data in reviews for f in data["findings"]]
        print(f"kikoeru-code-review: {len(reviews)} reviews, {counts_line(total)} -> {argv[1]}")
    else:
        print(__doc__, file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
