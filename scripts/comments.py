#!/usr/bin/env python3
"""Comment ratchet for Rust, Dart and WGSL sources.

    comments.py check              fail if any file has more or fewer comment
                                   lines than scripts/comment-baseline.json
    comments.py baseline           lower the baseline to what the tree has now
    comments.py list OUT [PATH..]  write every comment block, with its code, to OUT
    comments.py apply DECISIONS    delete the blocks a decisions file marks for
                                   deletion, then verify no code changed

Directives are not comments: `// ignore`, `// ignore_for_file`, `// coverage:`,
`// dart format`, `// @dart`, and any block holding a `SAFETY:` line or a
`# Safety` heading, which clippy and docs/14 require, and the one comment in a
Dart `catch` with nothing else in it, which `empty_catches` requires.
"""
import argparse
import hashlib
import json
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BASELINE = os.path.join(ROOT, "scripts", "comment-baseline.json")
LANGS = {".rs": "rust", ".dart": "dart", ".wgsl": "wgsl"}
SKIP_PARTS = (
    "frb_generated",
    ".g.dart",
    ".freezed.dart",
    "flutter_ui/lib/src/rust/",
    "flutter_ui/lib/l10n/app_localizations",
    "flutter_ui/third_party/",
    "flutter_ui/rust_builder/cargokit/",
    "generated_plugin_registrant",
)
DIRECTIVE = re.compile(
    r"^\s*//+\s*(ignore\b|ignore_for_file\b|ignore-start\b|ignore-end\b|coverage:|"
    r"dart format\b|@dart\b|rustfmt::)"
)
SAFETY = re.compile(r"SAFETY:|^\s*///\s*#\s*Safety\b")
IDENT = re.compile(r"[A-Za-z0-9_$]")
CATCH = re.compile(r"(?:catch\s*\([^)]*\)|\bon\s+[A-Za-z_][\w.<>?, ]*)\s*\{\s*$")
RAW_RUST =re.compile(r'(?:b|c)?r(#*)"')
NL = chr(10)
CR = chr(13)


def eol(src, i):
    j = src.find(NL, i)
    if j < 0:
        return len(src)
    return j - 1 if j > i and src[j - 1] == CR else j


def skip_block(src, i):
    depth, n = 0, len(src)
    while i < n:
        if src.startswith("/*", i):
            depth += 1
            i += 2
        elif src.startswith("*/", i):
            depth -= 1
            i += 2
            if depth == 0:
                return i
        else:
            i += 1
    return n


def skip_plain(src, i, quote):
    n = len(src)
    while i < n:
        if src[i] == chr(92):
            i += 2
        elif src[i] == quote:
            return i + 1
        else:
            i += 1
    return n


def lex_rust(src):
    spans, i, n = [], 0, len(src)
    while i < n:
        c = src[i]
        if c == "/" and src.startswith("//", i):
            j = eol(src, i)
            spans.append((i, j))
            i = j
        elif c == "/" and src.startswith("/*", i):
            j = skip_block(src, i)
            spans.append((i, j))
            i = j
        elif c == '"':
            i = skip_plain(src, i + 1, '"')
        elif c in "rbc" and (i == 0 or not IDENT.match(src[i - 1])):
            m = RAW_RUST.match(src, i)
            if m:
                end = '"' + m.group(1)
                j = src.find(end, m.end())
                i = n if j < 0 else j + len(end)
            else:
                i += 1
        elif c == "'":
            if src.startswith(chr(92), i + 1):
                j = i + 2
                while j < n and src[j] != "'" and src[j] != NL:
                    j += 1
                i = j + 1
            elif i + 2 < n and src[i + 2] == "'":
                i += 3
            else:
                i += 1
        else:
            i += 1
    return spans


def dart_string(src, i, quote, triple, raw):
    """Scan a Dart string body. Returns (index, 'end' | 'interp')."""
    n = len(src)
    close = quote * 3 if triple else quote
    while i < n:
        c = src[i]
        if not raw and c == chr(92):
            i += 2
        elif not raw and c == "$":
            if src.startswith("{", i + 1):
                return i + 2, "interp"
            i += 1
            while i < n and IDENT.match(src[i]) and src[i] != "$":
                i += 1
        elif src.startswith(close, i):
            return i + len(close), "end"
        elif c == NL and not triple:
            return i, "end"
        else:
            i += 1
    return n, "end"


def lex_dart(src):
    spans, i, n = [], 0, len(src)
    stack = []
    while i < n:
        c = src[i]
        if c == "/" and src.startswith("//", i):
            j = eol(src, i)
            spans.append((i, j))
            i = j
        elif c == "/" and src.startswith("/*", i):
            j = skip_block(src, i)
            spans.append((i, j))
            i = j
        elif c in "'\"":
            raw = i > 0 and src[i - 1] == "r" and (i < 2 or not IDENT.match(src[i - 2]))
            triple = src.startswith(c * 3, i)
            i += 3 if triple else 1
            i, how = dart_string(src, i, c, triple, raw)
            if how == "interp":
                stack.append([c, triple, 0])
        elif c == "{" and stack:
            stack[-1][2] += 1
            i += 1
        elif c == "}" and stack:
            if stack[-1][2] == 0:
                q, triple, _ = stack.pop()
                i, how = dart_string(src, i + 1, q, triple, False)
                if how == "interp":
                    stack.append([q, triple, 0])
            else:
                stack[-1][2] -= 1
                i += 1
        else:
            i += 1
    return spans


def lex_wgsl(src):
    spans, i, n = [], 0, len(src)
    while i < n:
        if src.startswith("//", i):
            j = eol(src, i)
            spans.append((i, j))
            i = j
        elif src.startswith("/*", i):
            j = skip_block(src, i)
            spans.append((i, j))
            i = j
        else:
            i += 1
    return spans


LEXERS = {"rust": lex_rust, "dart": lex_dart, "wgsl": lex_wgsl}


def blocks_of(path, src, keep_exempt=False):
    """Group comment spans into blocks. Each block is a dict with a line range."""
    lang = LANGS[os.path.splitext(path)[1]]
    spans = LEXERS[lang](src)
    starts = [0]
    for m in re.finditer(NL, src):
        starts.append(m.end())

    def line_of(off):
        lo, hi = 0, len(starts)
        while lo + 1 < hi:
            mid = (lo + hi) // 2
            if starts[mid] <= off:
                lo = mid
            else:
                hi = mid
        return lo + 1

    blocks = []
    for s, e in spans:
        line = line_of(s)
        lead = src[starts[line - 1] : s]
        standalone = lead.strip() == ""
        text = src[s:e]
        is_line = text.startswith("//")
        block = {
            "start": s,
            "end": e,
            "line": line,
            "end_line": line_of(max(s, e - 1)),
            "standalone": standalone,
            "line_comment": is_line,
            "text": text,
            "doc": text.startswith(("///", "//!", "/**", "/*!")),
            "directive": bool(DIRECTIVE.match(text)),
        }
        prev = blocks[-1] if blocks else None
        if (
            is_line
            and standalone
            and not block["directive"]
            and prev
            and prev["line_comment"]
            and prev["standalone"]
            and not prev["directive"]
            and prev["end_line"] + 1 == line
        ):
            prev["end"] = e
            prev["end_line"] = line
            prev["text"] = src[prev["start"] : e]
            continue
        blocks.append(block)
    out = []
    for b in blocks:
        b["exempt"] = (
            b["directive"]
            or any(SAFETY.search(x) for x in b["text"].split(NL))
            or (lang == "dart" and sole_catch_body(src, b))
        )
        if b["exempt"] and not keep_exempt:
            continue
        b["count"] = b["end_line"] - b["line"] + 1
        b["id"] = "%d.%s" % (b["line"], hashlib.sha1(b["text"].encode()).hexdigest()[:4])
        out.append(b)
    return out


def sole_catch_body(src, b):
    """The `empty_catches` lint wants a comment in an otherwise empty catch."""
    return bool(CATCH.search(src[max(0, b["start"] - 200) : b["start"]])) and src[b["end"] :].lstrip().startswith("}")


def tracked_files():
    run = subprocess.run(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    )
    files = []
    for p in run.stdout.splitlines():
        if os.path.splitext(p)[1] not in LANGS:
            continue
        if any(part in p for part in SKIP_PARTS):
            continue
        if not os.path.exists(os.path.join(ROOT, p)):
            continue
        files.append(p)
    return sorted(files)


def read(path):
    with open(os.path.join(ROOT, path), encoding="utf-8", newline="") as f:
        return f.read()


def is_generated(src):
    return "GENERATED CODE" in src[:400] or "@generated" in src[:400]


def counts():
    result = {}
    for p in tracked_files():
        src = read(p)
        if is_generated(src):
            continue
        c = sum(b["count"] for b in blocks_of(p, src))
        if c:
            result[p] = c
    return result


def load_baseline():
    if not os.path.exists(BASELINE):
        return {}
    with open(BASELINE, encoding="utf-8") as f:
        return json.load(f)


def write_baseline(data):
    lines = ["{"]
    items = sorted(data.items())
    for k, (path, c) in enumerate(items):
        lines.append("  %s: %d%s" % (json.dumps(path), c, "," if k + 1 < len(items) else ""))
    lines.append("}")
    with open(BASELINE, "w", encoding="utf-8", newline=NL) as f:
        f.write(NL.join(lines) + NL)


def cmd_check(_):
    now, base = counts(), load_baseline()
    worse, better = [], []
    for p in sorted(set(now) | set(base)):
        a, b = now.get(p, 0), base.get(p, 0)
        if a > b:
            worse.append("%s: %d comment lines, baseline %d" % (p, a, b))
        elif a < b:
            better.append("%s: %d comment lines, baseline %d" % (p, a, b))
    if worse:
        print("New comments. Say it in the code, or leave the comment out (docs/GUIDE.md):")
        print(NL.join(worse))
    if better:
        print("Fewer comments than the baseline. Run: python scripts/comments.py baseline")
        print(NL.join(better))
    if worse or better:
        return 1
    print("comments: %d files, %d lines, at baseline" % (len(now), sum(now.values())))
    return 0


def cmd_baseline(args):
    now, base = counts(), load_baseline()
    raised = [p for p in now if now[p] > base.get(p, 0)]
    if raised and not args.allow_increase:
        print("Refusing to raise the baseline for:")
        print(NL.join(raised))
        print("Delete those comments, or pass --allow-increase for a rename or split.")
        return 1
    write_baseline(now)
    print("baseline: %d files, %d lines" % (len(now), sum(now.values())))
    return 0


def context_of(src_lines, b):
    before = src_lines[b["line"] - 2].strip() if b["line"] >= 2 else ""
    after = []
    k = b["end_line"]
    while k < len(src_lines) and len(after) < 3:
        if src_lines[k].strip():
            after.append(src_lines[k].rstrip()[:140])
        k += 1
    return before[:140], after


def expand(paths):
    files = tracked_files()
    if not paths:
        return files
    wanted = []
    for p in paths:
        p = p.replace(chr(92), "/")
        if p in files:
            wanted.append(p)
        else:
            wanted += [f for f in files if f.startswith(p.rstrip("/") + "/")]
    return sorted(set(wanted))


def cmd_list(args):
    wanted = expand(args.paths)
    total = 0
    with open(args.out, "w", encoding="utf-8", newline=NL) as out:
        for p in wanted:
            src = read(p)
            if is_generated(src):
                continue
            blocks = blocks_of(p, src)
            if not blocks:
                continue
            lines = src.split(NL)
            out.write("=== %s (%d blocks)\n" % (p, len(blocks)))
            for b in blocks:
                before, after = context_of(lines, b)
                kind = ("trailing " if not b["standalone"] else "") + ("doc" if b["doc"] else "plain")
                out.write("[%s] L%d-%d %s\n" % (b["id"], b["line"], b["end_line"], kind))
                for t in b["text"].split(NL):
                    out.write("  | %s\n" % t.strip()[:240])
                if not b["standalone"]:
                    ls = src.rfind(NL, 0, b["start"]) + 1
                    out.write("  code: %s\n" % src[ls : b["start"]].strip()[:140])
                elif after:
                    out.write("  next: %s\n" % after[0])
            total += len(blocks)
    print("%d blocks in %d files" % (total, len(wanted)))
    return 0


def normalise(src, path):
    """Source with every comment blanked and whitespace collapsed."""
    lang = LANGS[os.path.splitext(path)[1]]
    parts, last = [], 0
    for s, e in LEXERS[lang](src):
        parts.append(src[last:s])
        parts.append(" ")
        last = e
    parts.append(src[last:])
    return re.sub(r"\s+", " ", "".join(parts)).strip()


def delete_blocks(src, chosen):
    """Remove the chosen blocks, bottom up. Returns the new source."""
    for b in sorted(chosen, key=lambda x: -x["start"]):
        s, e = b["start"], b["end"]
        ls = src.rfind(NL, 0, s) + 1
        le = src.find(NL, e)
        le = len(src) if le < 0 else le
        head, tail = src[ls:s], src[e:le]
        if head.strip() == "" and tail.strip() == "":
            src = src[:ls] + src[min(le + 1, len(src)) :]
            prev_blank = ls >= 2 and src[ls - 2] == NL
            next_blank = src[ls : ls + 1] in (NL, CR)
            if prev_blank and next_blank:
                nl = src.find(NL, ls)
                src = src[:ls] + src[nl + 1 :]
        elif head.strip() == "":
            src = src[:ls] + head + tail.lstrip(" \t") + src[le:]
        else:
            gap = ""
            if tail.strip() and not head.endswith((" ", "\t")) and not tail.startswith((" ", "\t")):
                gap = " "
            joined = head.rstrip(" \t") + gap + tail
            src = src[:ls] + joined.rstrip(" \t") + src[le:]
    return src


def cmd_apply(args):
    with open(args.decisions, encoding="utf-8") as f:
        decisions = json.load(f)
    deleted = 0
    problems = []
    for path, d in sorted(decisions.items()):
        if not os.path.exists(os.path.join(ROOT, path)):
            problems.append("%s: missing" % path)
            continue
        ids = set(d.get("tautology", []) + d.get("stale", []))
        ids |= {x["id"] if isinstance(x, dict) else x for x in d.get("excuse", [])}
        src = read(path)
        everything = {b["id"]: b for b in blocks_of(path, src, keep_exempt=True)}
        blocks = {i: b for i, b in everything.items() if not b["exempt"]}
        unknown = ids - set(everything)
        if unknown:
            problems.append("%s: unknown ids %s" % (path, sorted(unknown)[:5]))
        chosen = [blocks[i] for i in ids if i in blocks]
        if not chosen:
            continue
        new = delete_blocks(src, chosen)
        if normalise(new, path) != normalise(src, path):
            problems.append("%s: code changed, skipped" % path)
            continue
        with open(os.path.join(ROOT, path), "w", encoding="utf-8", newline="") as f:
            f.write(new)
        deleted += sum(b["count"] for b in chosen)
    for p in problems:
        print("PROBLEM", p)
    print("deleted %d comment lines" % deleted)
    return 1 if problems else 0


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("check").set_defaults(fn=cmd_check)
    b = sub.add_parser("baseline")
    b.add_argument("--allow-increase", action="store_true")
    b.set_defaults(fn=cmd_baseline)
    ls = sub.add_parser("list")
    ls.add_argument("out")
    ls.add_argument("paths", nargs="*")
    ls.set_defaults(fn=cmd_list)
    ap_ = sub.add_parser("apply")
    ap_.add_argument("decisions")
    ap_.set_defaults(fn=cmd_apply)
    args = ap.parse_args()
    sys.exit(args.fn(args))


if __name__ == "__main__":
    main()
