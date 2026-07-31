#!/usr/bin/env python3
"""RFC-0012 §3.5 — Responses API variant similarity audit.

Performs pairwise line-level similarity analysis on the 7 Responses API
implementation files. Produces a report identifying mergeable (shared) regions
vs. genuine vendor differences.

Normalization rules applied to each source line before comparison:
  - strip line comments (`// ...`) and block-comment remnants
  - remove leading/trailing whitespace and collapse inner runs
  - replace string literals with `"<STR>"`
  - replace numeric literals with `"<NUM>"`
  - leave identifiers intact (identifier renaming would over-merge unrelated code)

Pairwise similarity uses two metrics:
  - SequenceMatcher ratio (order-aware, accounts for block moves)
  - matched-line count (sum of matching block lengths)
"""

from __future__ import annotations

import difflib
import re
import sys
from dataclasses import dataclass
from pathlib import Path

FILES = [
    "aimux-providers/src/open_responses.rs",
    "aimux-providers/src/huggingface/responses.rs",
    "aimux-providers/src/azure/responses.rs",
    "aimux-providers/src/openai/responses/mod.rs",
    "aimux-providers/src/openai/responses/convert.rs",
    "aimux-providers/src/xai/responses/mod.rs",
    "aimux-providers/src/xai/responses/convert.rs",
]

# Patterns for normalization.
_LINE_COMMENT = re.compile(r"//.*$")
_STR_LITERAL = re.compile(r'"(?:[^"\\]|\\.)*"')
_RAW_STR = re.compile(r'r#*"(?:[^"]|"(?!#))*"#*"')
_NUM_LITERAL = re.compile(r"\b\d[\d_]*(\.\d+)?\b")
_WS = re.compile(r"\s+")

# Block-comment tracking is approximate: we drop lines that are entirely within
# a /* ... */ block. Rust doc comments (`///`, `//!`) are treated as comments
# too, since they are not executable code.


def strip_comments(text: str) -> list[str]:
    """Strip comments and doc comments, returning normalized code lines."""
    out: list[str] = []
    in_block = False
    for raw in text.splitlines():
        line = raw
        # Handle block comments spanning lines.
        if in_block:
            end = line.find("*/")
            if end == -1:
                continue
            line = line[end + 2 :]
            in_block = False
        # Remove block comments on a single line.
        line = re.sub(r"/\*.*?\*/", " ", line)
        # Detect an unterminated block comment start.
        start = line.find("/*")
        if start != -1:
            line = line[:start]
            in_block = True
        # Remove doc comments (/// and //!) and line comments.
        line = _LINE_COMMENT.sub("", line)
        line = re.sub(r"^\s*//[/!].*$", "", line)
        line = re.sub(r"^\s*//[/!]", "", line)
        if line.strip() == "":
            continue
        out.append(line)
    return out


def normalize_line(line: str) -> str:
    """Normalize a single code line for comparison."""
    s = line
    s = _RAW_STR.sub('"<STR>"', s)
    s = _STR_LITERAL.sub('"<STR>"', s)
    s = _NUM_LITERAL.sub("<NUM>", s)
    s = _WS.sub(" ", s).strip()
    return s


def load_normalized(path: Path) -> list[str]:
    text = path.read_text(encoding="utf-8")
    code_lines = strip_comments(text)
    return [normalize_line(l) for l in code_lines]


@dataclass
class PairResult:
    a: str
    b: str
    a_lines: int
    b_lines: int
    matched: int
    ratio: float


def compare(a_name: str, a_lines: list[str], b_name: str, b_lines: list[str]) -> PairResult:
    sm = difflib.SequenceMatcher(a=a_lines, b=b_lines, autojunk=False)
    matched = sum(m.size for m in sm.get_matching_blocks())
    ratio = sm.ratio()
    return PairResult(
        a=a_name,
        b=b_name,
        a_lines=len(a_lines),
        b_lines=len(b_lines),
        matched=matched,
        ratio=ratio,
    )


def jaccard(a: list[str], b: list[str]) -> float:
    sa, sb = set(a), set(b)
    if not sa and not sb:
        return 1.0
    return len(sa & sb) / len(sa | sb)


def main() -> int:
    repo = Path(__file__).resolve().parents[1]
    files = [(name, repo / name) for name in FILES]
    missing = [str(p) for _, p in files if not p.exists()]
    if missing:
        print("ERROR: missing files:", missing, file=sys.stderr)
        return 1

    norm: dict[str, list[str]] = {}
    raw_counts: dict[str, int] = {}
    for name, p in files:
        norm[name] = load_normalized(p)
        raw_counts[name] = sum(1 for _ in p.read_text(encoding="utf-8").splitlines())

    out: list[str] = []
    out.append("# RFC-0012 §3.5 — Responses API 相似度审计报告\n")
    out.append(f"工作目录: `{repo}`\n")
    out.append("## 1. 文件规模\n")
    out.append("| 文件 | 原始行数 | 归一化后代码行（去注释/空白） |")
    out.append("|---|---:|---:|")
    for name, _ in files:
        out.append(f"| `{name}` | {raw_counts[name]} | {len(norm[name])} |")
    total_raw = sum(raw_counts.values())
    total_norm = sum(len(v) for v in norm.values())
    out.append(f"| **合计** | **{total_raw}** | **{total_norm}** |")

    out.append("\n## 2. 两两相似度（归一化后，顺序敏感 SequenceMatcher）\n")
    out.append("| A | B | A行 | B行 | 匹配行数 | 相似度 |")
    out.append("|---|---|---:|---:|---:|---:|")
    results: list[PairResult] = []
    for i in range(len(files)):
        for j in range(i + 1, len(files)):
            na, nb = files[i][0], files[j][0]
            r = compare(na, norm[na], nb, norm[nb])
            results.append(r)
            out.append(
                f"| `{na}` | `{nb}` | {r.a_lines} | {r.b_lines} | {r.matched} | {r.ratio*100:.1f}% |"
            )

    out.append("\n## 3. 两两相似度（归一化后，集合 Jaccard，忽略顺序）\n")
    out.append("| A | B | Jaccard |")
    out.append("|---|---|---:|")
    for i in range(len(files)):
        for j in range(i + 1, len(files)):
            na, nb = files[i][0], files[j][0]
            j_val = jaccard(norm[na], norm[nb])
            out.append(f"| `{na}` | `{nb}` | {j_val*100:.1f}% |")

    # Average similarity of each file to the others (by matched-line count ratio).
    out.append("\n## 4. 各文件与其余文件的平均共享度\n")
    out.append("| 文件 | 平均相似度(对较小文件) | 平均 Jaccard |")
    out.append("|---|---:|---:|")
    for idx in range(len(files)):
        name = files[idx][0]
        ratios = []
        jaccs = []
        for jdx in range(len(files)):
            if jdx == idx:
                continue
            other = files[jdx][0]
            sm = difflib.SequenceMatcher(
                a=norm[name], b=norm[other], autojunk=False
            )
            matched = sum(m.size for m in sm.get_matching_blocks())
            denom = min(len(norm[name]), len(norm[other])) or 1
            ratios.append(matched / denom)
            jaccs.append(jaccard(norm[name], norm[other]))
        out.append(
            f"| `{name}` | {sum(ratios)/len(ratios)*100:.1f}% | {sum(jaccs)/len(jaccs)*100:.1f}% |"
        )

    out.append("\n## 5. 结论：可合并部分 vs 真实差异\n")
    out.append("基于上面的相似度矩阵，识别出以下结构性观察：\n")
    out.append(
        "- **mod.rs（openai/xai）与 azure/responses.rs、open_responses.rs、huggingface/responses.rs** "
        "都实现了 `LanguageModel` trait 的 `do_generate` / `do_stream`，"
        "其流式事件解析主循环（`response.created -> output_item.added -> output_text.delta -> "
        "output_text.done -> output_item.done -> response.completed`）结构高度同构，"
        "差异主要在：endpoint 拼接、header 构造、个别事件名/字段名。"
    )
    out.append(
        "- **convert.rs（openai/xai）与各 responses 文件中的 build_request_body / convert_to_*_input** "
        "承担请求体构建与 input 转换，结构同构但厂商字段（reasoning、metadata、provider options）有真实差异。"
    )
    out.append(
        "- **usage 提取**（`extract_usage` / `convert_usage` / `convert_responses_usage`）和 "
        "**finish_reason 映射**（`map_*_finish_reason`）是最易合并的小函数。"
    )
    out.append(
        "- **媒体类型解析**（base64 data URL 拆分、top-level media type）在 huggingface/azure 重复实现，可共享。"
    )
    out.append(
        "- 真实差异（必须保留为厂商覆盖）：endpoint/base_url、model id 校验与映射、"
        "provider-specific provider options（openai 的 item_id/phase/namespace、xai 的 source/tool 解析、"
        "azure 的 deployment 前缀注入、huggingface 的消息格式）。"
    )
    out.append(
        "- **不强行合并到单一函数**：各厂商 responses 实现有真实协议差异，只提取共享框架，"
        "差异以厂商覆盖形式保留。"
    )

    out.append("\n## 6. 合并策略（对应 RFC §3.5 步骤 1-2）\n")
    out.append(
        "1. 在 `openai/responses/responses_convert.rs` 提取共享框架："
        "请求体构建通用片段、流式事件解析通用片段、usage 提取通用片段、"
        "媒体类型 data-URL 拆分 helper。"
    )
    out.append(
        "2. 各厂商只保留差异覆盖：endpoint 拼接、model id 映射、provider-specific 字段，"
        "调用共享框架函数。"
    )

    report_path = repo / "docs" / "audit" / "responses-similarity.md"
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text("\n".join(out) + "\n", encoding="utf-8")
    print(f"wrote {report_path}")
    print()
    # Also print the pairwise table to stdout for the log.
    print("\n".join(out))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
