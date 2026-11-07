"""Check the contextual (Digital Khatt elongated) Mushaf data on every page.

Compares the bundled pages and precomputed outlines with independent sources:
the original Digital Khatt 15-line layout and word databases, and the QUL word
list. It reads files only and changes nothing.

    python verify_contextual.py <Resources folder> <project root> [report.json]
"""
import hashlib, json, os, sqlite3, sys
from collections import Counter

res, root = sys.argv[1], sys.argv[2]
report_path = sys.argv[3] if len(sys.argv) > 3 else None
dk = os.path.join(res, "experimental", "digital-khatt")
raw = os.path.join(root, "work", "digital-khatt", "raw")
problems = []
stats = Counter()
def problem(page, line, what):
    problems.append({"page": page, "line": line, "problem": what})

manifest = json.load(open(os.path.join(dk, "shaped", "manifest.json"), encoding="utf8"))
glyph_table = json.load(open(os.path.join(dk, "shaped", "glyphs.json"), encoding="utf8"))
glyph_ids = set(glyph_table) if isinstance(glyph_table, dict) else set(map(str, range(len(glyph_table))))

layout = sqlite3.connect(os.path.join(raw, "digital-khatt-15-lines.db"))
rows = {}
for page, line, kind, centered, first, last, surah in layout.execute(
        "select page_number, line_number, line_type, is_centered, first_word_id, last_word_id, surah_number from pages"):
    rows[(page, line)] = (kind, bool(centered), first if first != "" else None, last if last != "" else None)
words_db = sqlite3.connect(os.path.join(raw, "digital-khatt-v2.db"))
dk_text = {i: (t, s, a) for i, t, s, a in words_db.execute("select id, text, surah, ayah from words")}
qul = {w["id"]: (w["surah"], w["ayah"]) for w in json.load(open(os.path.join(root, "data", "qul", "processed", "words.json"), encoding="utf8"))}

seen_words = []
scales, full, centered_lines, scaled_lines = [], 0, 0, []
for n in range(1, 605):
    page_bytes = open(os.path.join(dk, "pages", f"p{n}.json"), "rb").read()
    page = json.loads(page_bytes)
    if hashlib.sha256(page_bytes).hexdigest() == manifest["sourceHashes"].get(str(n)):
        stats["source hash matches outlines manifest"] += 1
    else:
        stats["source hash differs from outlines manifest"] += 1
    shaped = json.load(open(os.path.join(dk, "shaped", f"p{n}.json"), encoding="utf8"))["lines"]
    page_rows = {k[1]: v for k, v in rows.items() if k[0] == n}
    if len(page["lines"]) != len(page_rows):
        problem(n, None, f"{len(page['lines'])} lines, original layout has {len(page_rows)}")
    for line in page["lines"]:
        ln = line["lineNumber"]
        kind, centered, first, last = page_rows.get(ln, (None, None, None, None))
        stats["lines"] += 1
        if kind != line["type"] or centered != line["isCentered"]:
            problem(n, ln, f"type/centering {line['type']}/{line['isCentered']} vs original {kind}/{centered}")
        if line["type"] != "ayah":
            stats[f"{line['type']} lines (not elongated)"] += 1
            continue
        if (line["firstWordId"], line["lastWordId"]) != (first, last):
            problem(n, ln, f"words {line['firstWordId']}-{line['lastWordId']} vs original {first}-{last}")
        ids = [w["id"] for w in line["words"]]
        if ids != list(range(line["firstWordId"], line["lastWordId"] + 1)):
            problem(n, ln, "word ids are not the line's complete range")
        for w in line["words"]:
            seen_words.append(w["id"])
            t, s, a = dk_text.get(w["id"], (None, None, None))
            if w["text"] != t:
                problem(n, ln, f"word {w['id']} text differs from the original Digital Khatt database")
            if (w["surah"], w["ayah"]) != (s, a) or qul.get(w["id"]) != (s, a):
                problem(n, ln, f"word {w['id']} surah/ayah differs between sources")
        s = shaped.get(str(ln))
        if s is None:
            if n <= 2:
                stats["opening-page lines drawn with the original font"] += 1
            else:
                problem(n, ln, "no elongated outline for this ayah line")
            continue
        stats["elongated lines"] += 1
        bounds = {int(k): v for k, v in s["wordBounds"].items()}
        if sorted(bounds) != ids:
            problem(n, ln, "outline words differ from the line's words")
        glyph_words = Counter(g[3] for g in s["glyphs"])
        if any(glyph_words[i] == 0 for i in ids) or set(glyph_words) - set(ids):
            problem(n, ln, "a word has no glyphs, or glyphs belong to another line")
        if any(str(g[0]) not in glyph_ids for g in s["glyphs"]):
            problem(n, ln, "glyph outside the outline table")
        # Right-to-left: each next word lies to the left of the previous one.
        width = s["width"]
        for a_id, b_id in zip(ids, ids[1:]):
            overlap = bounds[b_id][1] - bounds[a_id][0]
            if overlap > 0.02 * width:
                problem(n, ln, f"words {a_id} and {b_id} overlap by {overlap / width:.1%} of the line")
        # Ink width after the line's own scale, as the renderer draws it.
        left, right = s["bounds"][0], s["bounds"][2]
        span = (right - left) * s["scale"] / width
        scaled_lines.append((s["scale"], n, ln))
        scales.append(s["scale"])
        if line["isCentered"]:
            centered_lines += 1
        elif span < 0.97:
            problem(n, ln, f"justified line fills only {span:.1%} of its width")
        else:
            full += 1
        if span > 1.03:
            problem(n, ln, f"line ink extends to {span:.1%} of its width")

dupes = [i for i, c in Counter(seen_words).items() if c > 1]
missing = sorted(set(dk_text) - set(seen_words))
summary = {
    "pages": 604,
    "lines": stats["lines"],
    "originalLayoutLines": len(rows),
    "elongatedAyahLines": stats["elongated lines"],
    "justifiedFullWidth": full,
    "centeredAyahLines": centered_lines,
    "otherLines": {k: v for k, v in stats.items() if "not elongated" in k},
    "wordsPlaced": len(seen_words),
    "wordsInDatabase": len(dk_text),
    "wordsMissing": len(missing),
    "wordsDuplicated": len(dupes),
    "scaleMin": round(min(scales), 4), "scaleMax": round(max(scales), 4),
    "openingPageLines": stats["opening-page lines drawn with the original font"],
    "lineScale": {"below 0.80": sum(1 for x in scales if x < 0.80), "0.80-0.90": sum(1 for x in scales if 0.80 <= x < 0.90),
                  "0.90-0.95": sum(1 for x in scales if 0.90 <= x < 0.95), "0.95 and above": sum(1 for x in scales if x >= 0.95)},
    "mostReducedLines": [{"page": p, "line": int(l), "scale": round(sc, 3)} for sc, p, l in sorted(scaled_lines)[:12]],
    "sourceHashes": {k: v for k, v in stats.items() if "source hash" in k},
    "problems": len(problems),
}
print(json.dumps(summary, indent=2, ensure_ascii=False))
for p in problems[:25]:
    print(p)
if report_path:
    json.dump({"summary": summary, "problems": problems, "missingWordIds": missing[:200]},
              open(report_path, "w", encoding="utf8"), indent=2, ensure_ascii=False)
