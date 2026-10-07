#!/usr/bin/env python3
"""What no suite executes, from the coverage run's JSON export and its
missing-lines report (scripts/coverage.sh --arms): the compiler's and the
runtime's files by missed lines, the functions that never ran, the table's
rows whose runtime function never ran in either mode, and the uncovered
lines of the backends classified — a refusal, a match arm, or a plain
statement. An arm no suite reaches is where a lowering slip hides (design
rule 22; docs/bug-classes.md).

Usage: scripts/coverage_arms.py <coverage.json> <missing.txt> > arms.txt
"""
import io, json, re, subprocess, sys, os
from collections import defaultdict

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
KEY = ["src/parser.rs", "src/lexer.rs", "src/lower.rs", "src/type_infer.rs", "src/type_infer/rules.rs", "src/type_infer/calls.rs",
       "src/lower_ir.rs", "src/lower_ir/checks.rs", "src/lower_ir/frames.rs", "src/ir_verify.rs", "src/ir_interp.rs", "src/ops.rs",
       "src/emit_rs/exprs.rs", "src/emit_rs/stmts.rs", "src/emit_rs/program.rs", "src/emit_rs/builtins.rs", "src/checker.rs", "src/table.rs"]


def demangle(names):
    """Demangled names, through rustfilt or c++filt when one is there."""
    for tool in ("rustfilt", "c++filt"):
        try:
            out = subprocess.run([tool], input="\n".join(names) + "\n", capture_output=True, text=True, check=True).stdout
            got = out.split("\n")
            if len(got) >= len(names):
                return dict(zip(names, got))
        except (OSError, subprocess.CalledProcessError):
            continue
    return {n: n for n in names}


def fold(n):
    return re.sub(r"::\{closure#\d+\}", "", re.sub(r"::\{\{closure\}\}.*", "", n))


def rel(p):
    return p[len(REPO) + 1:] if p.startswith(REPO + "/") else p


def main(json_path, missing_path):
    data = json.load(io.open(json_path, encoding="utf-8"))["data"][0]
    files = {rel(f["filename"]): f["summary"] for f in data["files"]}
    funcs = data["functions"]
    names = demangle(sorted(set(f["name"] for f in funcs)))

    rows = []
    for r, s in files.items():
        if not r.startswith(("src/", "crow_runtime/", "crow_web/", "crow_db/")):
            continue
        L, F = s["lines"], s["functions"]
        rows.append((L["count"] - L["covered"], L["count"], F["count"] - F["covered"], F["count"], r))
    rows.sort(reverse=True)
    print("== files by lines no suite reached (missed/lines, missed functions/functions)")
    for ml, l, mf, f, r in rows[:40]:
        print("%6d/%-6d %4d/%-4d %5.1f%%  %s" % (ml, l, mf, f, 100.0 * ml / l if l else 0, r))

    never, ran = defaultdict(set), set()
    counts = defaultdict(int)
    for fn in funcs:
        short = fold(names.get(fn["name"], fn["name"]))
        counts[short] += fn["count"]
        for p in fn["filenames"]:
            if fn["count"] == 0:
                never[rel(p)].add(short)
            else:
                ran.add(short)
    print("\n== functions no suite ran (closures folded into their function)")
    for k in KEY:
        cold = sorted(n for n in never.get(k, ()) if n not in ran)
        if cold:
            print("-- %s: %d" % (k, len(cold)))
            for n in cold:
                print("     " + n.replace("crow::", "", 1))

    table = io.open(os.path.join(REPO, "src/table.rs"), encoding="utf-8").read().split("\n")
    row_re = re.compile(r'row!\("([^"]+)",\s*"([^"]*)",\s*"([^"]+)",\s*([A-Za-z0-9_:]+)')
    cold, absent, total = [], [], 0
    for line in table:
        m = row_re.search(line)
        if not m:
            continue
        total += 1
        rid, path = m.group(1), m.group(4)
        c = counts.get(path)
        if c is None:
            last, mod = path.rsplit("::", 1)[-1], path.rsplit("::", 1)[0].split("::")[-1]
            hits = [k for k in counts if re.sub(r"<[^<>]*>", "", k).endswith("::" + last) and mod in k]
            c = sum(counts[k] for k in hits) if hits else None
        if c is None:
            absent.append(rid)
        elif c == 0:
            cold.append("%s (%s)" % (rid, path))
    print("\n== rows whose runtime function never ran in either mode: %d of %d" % (len(cold), total))
    for u in cold:
        print("     " + u)
    print("(%d rows name a generic or inlined function the profile cannot match)" % len(absent))

    missing = io.open(missing_path, encoding="utf-8").read()
    print("\n== the uncovered lines of the backends and the front end, classified")
    for f in ["src/ops.rs", "src/ir_interp.rs", "src/emit_rs/exprs.rs", "src/emit_rs/builtins.rs", "src/lower_ir.rs", "src/ir_verify.rs", "src/parser.rs", "src/lexer.rs"]:
        m = re.search(r"^(?:\S*/)?" + re.escape(f) + r":\s*(.*)$", missing, re.M)
        if not m:
            continue
        lines = set()
        for part in m.group(1).split(","):
            part = part.strip()
            if "-" in part:
                a, b = part.split("-")
                lines.update(range(int(a), int(b) + 1))
            elif part:
                lines.add(int(part))
        src = io.open(os.path.join(REPO, f), encoding="utf-8").read().split("\n")
        kinds, arms = defaultdict(int), []
        for l in sorted(lines):
            t = src[l - 1].strip() if l - 1 < len(src) else ""
            if re.search(r'Err\(|refus|expected |cannot |crow_stop|stops::|panic|unreachable|found\(', t):
                k = "a refusal or an error path"
            elif re.match(r'^\(?(Value|Ty|Expr|Stmt|Kind|Arg|Convert|Target|Cell|Some\(Value|"[a-z_]+")', t):
                k = "a match arm"
                arms.append("%d: %s" % (l, t[:110]))
            elif t.startswith("//") or t in ("}", "};", "{", ")", "),", "]", "],"):
                k = "a brace or a comment"
            else:
                k = "a statement"
            kinds[k] += 1
        print("-- %s: %d lines: %s" % (f, len(lines), ", ".join("%s %d" % (k, n) for k, n in sorted(kinds.items(), key=lambda x: -x[1]))))
        for a in arms:
            print("     " + a)


if __name__ == "__main__":
    if len(sys.argv) != 3:
        print(__doc__)
        sys.exit(2)
    main(sys.argv[1], sys.argv[2])
