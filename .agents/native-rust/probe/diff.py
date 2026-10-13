"""Compare two probe JSONL files (old vs new). Usage: python3 -I diff.py old.jsonl new.jsonl [max_examples]"""
import json
import sys
from collections import Counter, defaultdict


def load(p):
    out = {}
    for line in open(p):
        d = json.loads(line)
        out[d["key"]] = d
    return out


def first_diff(a, b, path=""):
    if type(a) is not type(b):
        return path, a, b
    if isinstance(a, dict):
        for k in sorted(set(a) | set(b)):
            if k not in a or k not in b:
                return f"{path}.{k}", a.get(k, "<absent>"), b.get(k, "<absent>")
            r = first_diff(a[k], b[k], f"{path}.{k}")
            if r:
                return r
        return None
    if isinstance(a, list):
        for i, (x, y) in enumerate(zip(a, b)):
            r = first_diff(x, y, f"{path}[{i}]")
            if r:
                return r
        if len(a) != len(b):
            return f"{path}.len", len(a), len(b)
        return None
    return None if a == b else (path, a, b)


def main():
    old, new = load(sys.argv[1]), load(sys.argv[2])
    maxex = int(sys.argv[3]) if len(sys.argv) > 3 else 3
    keys = sorted(set(old) | set(new))
    only_old = [k for k in keys if k not in new]
    only_new = [k for k in keys if k not in old]
    both = [k for k in keys if k in old and k in new]
    req_n = sum(len(old[k]["requests"]) for k in both)
    part_seqs = sum(1 for k in both if "parts" in old[k]["outcome"])
    parts_n = sum(len(old[k]["outcome"]["parts"]) for k in both if "parts" in old[k]["outcome"])
    unstable = [k for k in both if "unstable_b" in old[k] or "unstable_b" in new[k]]
    classes = defaultdict(list)
    req_diff = parts_diff = out_diff = 0
    for k in both:
        a, b = old[k], new[k]
        if a == b:
            continue
        if a["requests"] != b["requests"]:
            req_diff += 1
            r = first_diff(a["requests"], b["requests"], "requests")
        else:
            out_diff += 1
            if "parts" in a["outcome"]:
                parts_diff += 1
            r = first_diff(a["outcome"], b["outcome"], "outcome")
        import re
        cls = (k.split("|")[1], re.sub(r"\[\d+\]", "[]", r[0]) if r else "?")
        classes[cls].append((k, r))
    s_old = Counter(("err" if "err" in old[k]["outcome"] else "parts" if "parts" in old[k]["outcome"] else "ok" if "ok" in old[k]["outcome"] else "other") for k in both)
    print(f"ops: old={len(old)} new={len(new)} compared={len(both)} only_old={len(only_old)} only_new={len(only_new)}")
    print(f"requests compared: {req_n}; stream-part sequences compared: {part_seqs} ({parts_n} parts)")
    print(f"outcome kinds (old): {dict(s_old)}")
    print(f"unstable in-process: {len(unstable)}")
    print(f"ops differing: {sum(len(v) for v in classes.values())} (request diffs: {req_diff}, outcome diffs: {out_diff}, of which stream-part seqs: {parts_diff})")
    for cls, items in sorted(classes.items(), key=lambda kv: -len(kv[1])):
        print(f"\n== class route={cls[0]} first-diff-path={cls[1]} count={len(items)}")
        for k, r in items[:maxex]:
            print(f"  {k}")
            if r:
                print(f"    old: {json.dumps(r[1])[:400]}")
                print(f"    new: {json.dumps(r[2])[:400]}")
    for k in only_old[:5]:
        print("only old:", k)
    for k in only_new[:5]:
        print("only new:", k)


main()
