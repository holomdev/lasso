"""Check the README's Markdown fences are balanced and its headings are outside them.

An unclosed ``` swallows everything after it, which is how `## Roadmap` stopped being a
heading and became code. This checks that every fence closes, that no heading is trapped
inside one, and that the file ends outside a fence.
"""

import pathlib
import re

path = pathlib.Path("README.md")
lines = path.read_text(encoding="utf-8").splitlines()

inside = False
opened_at = 0
trapped = []
problems = []

for number, line in enumerate(lines, start=1):
    stripped = line.strip()
    if stripped.startswith("```"):
        if inside:
            inside = False
        else:
            inside = True
            opened_at = number
        continue
    if inside and stripped.startswith("#"):
        trapped.append((number, stripped, opened_at))

if inside:
    problems.append(f"a fence opened at line {opened_at} is never closed")

for number, heading, opened_at in trapped:
    problems.append(
        f"line {number} is a heading inside the fence opened at line {opened_at}: {heading}"
    )

fences = sum(1 for line in lines if line.strip().startswith("```"))
print(f"README.md: {len(lines)} lines, {fences} fence markers, {fences // 2} blocks")

if problems:
    print("PROBLEMS:")
    for problem in problems:
        print(f"  - {problem}")
    raise SystemExit(1)

print("every fence closes, and no heading is trapped inside one")

# The headings, so a reader can see the structure is intact.
headings = [line for line in lines if re.match(r"^#{1,3} ", line)]
print(f"{len(headings)} headings, from {headings[0]!r} to {headings[-1]!r}")
