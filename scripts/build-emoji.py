#!/usr/bin/env python3
"""Write the emoji picker's data, app/Sources/Amgi/Emoji.tsv, from GitHub's gemoji (MIT).

    scripts/build-emoji.py [path/to/emoji.json]

Without a path, downloads the latest db/emoji.json from github/gemoji.
Each category starts with an `@Category` line, followed by one line per emoji:
`emoji<TAB>name<TAB>shortcodes<TAB>keywords`, with shortcodes and keywords separated
by spaces.
"""

import json
import pathlib
import sys
import urllib.request

SOURCE = "https://raw.githubusercontent.com/github/gemoji/master/db/emoji.json"
OUT = pathlib.Path(__file__).resolve().parent.parent / "app/Sources/Amgi/Emoji.tsv"


def main() -> None:
    if len(sys.argv) > 1:
        data = json.loads(pathlib.Path(sys.argv[1]).read_text())
    else:
        with urllib.request.urlopen(SOURCE) as response:
            data = json.load(response)

    lines = []
    category = None
    for entry in data:
        if entry["category"] != category:
            category = entry["category"]
            lines.append(f"@{category}")
        name = entry["description"].strip()
        aliases = " ".join(alias.lower() for alias in entry["aliases"])
        tags = " ".join(dict.fromkeys(tag.lower() for tag in entry["tags"]))
        lines.append(f"{entry['emoji']}\t{name}\t{aliases}\t{tags}")

    OUT.write_text("\n".join(lines) + "\n")
    print(f"Wrote {len(data)} emoji to {OUT.relative_to(OUT.parents[3])}.")


if __name__ == "__main__":
    main()
