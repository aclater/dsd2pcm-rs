"""Pack a directory into a zip with fixed order, times and permissions (reproducible).

    python -I .github/scripts/pack-zip.py OUT.zip DIR MTIME

The archive holds DIR's files under DIR's own name; MTIME (seconds since the epoch, the
commit's time) is every entry's time. Standard library only.
"""

import os
import sys
import time
import zipfile


def main():
    out, root, mtime = sys.argv[1], sys.argv[2], int(sys.argv[3])
    stamp = time.gmtime(max(mtime, 315532800))[:6]  # zip times start in 1980
    base = os.path.basename(os.path.normpath(root))
    files = []
    for d, _, names in os.walk(root):
        for n in names:
            files.append(os.path.join(d, n))
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for f in sorted(files, key=lambda p: os.path.relpath(p, root).replace(os.sep, "/")):
            rel = os.path.relpath(f, root).replace(os.sep, "/")
            info = zipfile.ZipInfo(f"{base}/{rel}", date_time=stamp)
            info.external_attr = (0o755 if rel.endswith(".exe") else 0o644) << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            with open(f, "rb") as fh:
                z.writestr(info, fh.read())


if __name__ == "__main__":
    main()
