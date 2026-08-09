"""Read the built PDF back and refuse a render that lost the documentation.

mkdocs-with-pdf reports success as long as it wrote a file. It said so while producing an
eight-page book out of a hundred-and-thirty-page site, because one CSS rule clipped the
document — no error, no warning, a valid PDF that happened to be a tenth of itself. The
only way to catch that class of failure is to open the result and look.

    python tools/check_pdf.py site/pdf/betterinstaller.pdf --min-pages 25 \
        --expect "BetterInstaller" --expect ".bpkg"
"""

import argparse
import sys
from pathlib import Path


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("pdf", type=Path)
    ap.add_argument("--min-pages", type=int, default=20,
                    help="fail below this many pages (the truncation symptom)")
    ap.add_argument("--expect", action="append", default=[],
                    help="text that must appear somewhere in the PDF; repeatable")
    args = ap.parse_args()

    if not args.pdf.is_file():
        print(f"::error::{args.pdf} was not produced")
        return 1

    size_kb = args.pdf.stat().st_size / 1024
    try:
        from pypdf import PdfReader
    except ImportError:
        print("::error::pypdf is not installed — this check cannot run, so it must not pass")
        return 1

    reader = PdfReader(str(args.pdf))
    pages = len(reader.pages)
    print(f"{args.pdf}: {pages} pages, {size_kb:.0f} KB")

    failed = False
    if pages < args.min_pages:
        print(f"::error::only {pages} pages, expected at least {args.min_pages} — "
              "the render is almost certainly truncated (check html/body height in the print CSS)")
        failed = True

    if args.expect:
        # Whitespace is stripped from BOTH sides before comparing: some fonts make pypdf
        # return per-glyph spacing, so "BetterInstaller" comes back as "B e t t e r…" and a
        # naive substring test fails on a PDF that is perfectly fine.
        haystack = "".join((p.extract_text() or "") for p in reader.pages)
        haystack = "".join(haystack.split())
        for needle in args.expect:
            if "".join(needle.split()) not in haystack:
                print(f"::error::expected text not found in the PDF: {needle!r}")
                failed = True
            else:
                print(f"  found: {needle!r}")

    if failed:
        return 1
    print("PDF looks complete")
    return 0


if __name__ == "__main__":
    sys.exit(main())
