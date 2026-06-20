#!/usr/bin/env python3
"""Mirror the mezon-react translation corpus into assets/i18n/<locale>.json.

Each React namespace file (libs/translations/src/languages/<lng>/<ns>.json) is
flattened to dot-keys prefixed by the namespace ("clan.title", "setting.language.title",
arrays become "key.0", "key.1", ...) and merged per locale. React is authoritative
for any key it defines; keys already present in the desktop file but absent from React
(desktop-specific strings: login.*, root.*, dm.*, chat.*, setting.{account,devices,
voice,profile,clanProfile,...}, language autonyms) are preserved.

The desktop i18n runtime (crates/mezon-i18n) reads these files via include_str!, so a
key is available to t(locale, key) as soon as it exists here.

Usage:
    python3 scripts/sync_i18n.py
    MEZON_TRANSLATIONS_DIR=/path/to/mezon/libs/translations/src/languages python3 scripts/sync_i18n.py
"""
import json
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DESK = os.path.join(ROOT, "assets", "i18n")
REACT = os.environ.get(
    "MEZON_TRANSLATIONS_DIR",
    os.path.join(ROOT, "..", "mezon", "libs", "translations", "src", "languages"),
)
LOCALES = ["en", "vi", "ru", "es", "tt", "de", "it", "pt", "jpn", "kr", "swe"]


def flatten(node, prefix, out):
    if isinstance(node, dict):
        for k, v in node.items():
            flatten(v, f"{prefix}.{k}" if prefix else str(k), out)
    elif isinstance(node, list):
        for i, v in enumerate(node):
            flatten(v, f"{prefix}.{i}" if prefix else str(i), out)
    elif isinstance(node, str):
        out[prefix] = node
    else:
        out[prefix] = json.dumps(node, ensure_ascii=False)


def react_flat(locale):
    out = {}
    import glob

    for f in sorted(glob.glob(os.path.join(REACT, locale, "*.json"))):
        ns = os.path.splitext(os.path.basename(f))[0]
        with open(f, encoding="utf-8") as fh:
            flatten(json.load(fh), ns, out)
    return out


def main():
    if not os.path.isdir(REACT):
        sys.exit(f"React translations dir not found: {REACT}\nSet MEZON_TRANSLATIONS_DIR.")
    for locale in LOCALES:
        react = react_flat(locale)
        path = os.path.join(DESK, locale + ".json")
        existing = json.load(open(path, encoding="utf-8")) if os.path.exists(path) else {}
        merged = dict(existing)
        merged.update(react)
        bad = [k for k, v in merged.items() if not isinstance(v, str)]
        assert not bad, f"{locale}: non-string values {bad[:5]}"
        ordered = {k: merged[k] for k in sorted(merged)}
        with open(path, "w", encoding="utf-8") as fh:
            json.dump(ordered, fh, ensure_ascii=False, indent=2)
            fh.write("\n")
        preserved = len(set(existing) - set(react))
        print(f"{locale}: {len(ordered)} keys (react={len(react)}, preserved={preserved})")


if __name__ == "__main__":
    main()
