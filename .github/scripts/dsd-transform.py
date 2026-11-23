"""Release test files (docs/release-test-plan.md), made from real DSF and DFF files:
the same audio in the other container, and DSF copies whose sample count is not a
multiple of 8. Writes only OUT; the input is opened read-only.

    python3 -I .github/scripts/dsd-transform.py dsf2dff IN.dsf OUT.dff
    python3 -I .github/scripts/dsd-transform.py dff2dsf IN.dff OUT.dsf
    python3 -I .github/scripts/dsd-transform.py odd IN.dsf OUT.dsf DROP [IDLE_BYTES]
    python3 -I .github/scripts/dsd-transform.py head IN.dsf OUT.dsf SECONDS

`odd` copies IN with its sample count lowered by DROP (1 to 7), so the last byte of each
channel ends in DROP padding bits, sets those bits to zero (the case that clicked before
0.1.2), and with IDLE_BYTES first overwrites each channel's last IDLE_BYTES bytes with
the idle pattern 0x69 (a silent end). `head` copies the first SECONDS (an excerpt for
slow tools such as valgrind). DSD64 only. Standard library only.
"""

import struct
import sys

BLOCK = 4096
REV = bytes(int(f"{i:08b}"[::-1], 2) for i in range(256))
DFF_IDS = {
    1: [b"C   "],
    2: [b"SLFT", b"SRGT"],
    5: [b"MLFT", b"MRGT", b"C   ", b"LS  ", b"RS  "],
    6: [b"MLFT", b"MRGT", b"C   ", b"LFE ", b"LS  ", b"RS  "],
}
DSF_TYPE = {1: 1, 2: 2, 5: 6, 6: 7}


def read_dsf(f):
    head = f.read(92)
    if head[:4] != b"DSD " or head[28:32] != b"fmt ":
        sys.exit("not a DSF file")
    ch, fs, bits, samples, block = struct.unpack_from("<IIIQI", head, 52)
    if head[80:84] != b"data" or fs != 2822400 or bits != 1 or block != BLOCK:
        sys.exit(f"DSF: need DSD64, LSB first, {BLOCK}-byte blocks")
    return ch, samples


def dsf_blocks(f, ch, nbytes):
    """Per block group: one bytes object per channel, MSB first, trimmed to nbytes."""
    f.seek(92)
    done = 0
    while done < nbytes:
        group = f.read(BLOCK * ch)
        take = min(BLOCK, nbytes - done)
        yield [group[c * BLOCK : c * BLOCK + take].translate(REV) for c in range(ch)]
        done += take


def dsf2dff(src, dst):
    with open(src, "rb") as f:
        ch, samples = read_dsf(f)
        if samples % 8:
            sys.exit("dsf2dff: sample count not a multiple of 8")
        n = samples // 8
        prop = b"SND "
        prop += b"FS  " + struct.pack(">QI", 4, 2822400)
        ids = b"".join(DFF_IDS[ch])
        prop += b"CHNL" + struct.pack(">QH", 2 + len(ids), ch) + ids
        prop += b"CMPR" + struct.pack(">Q", 20) + b"DSD " + bytes([14]) + b"not compressed" + b"\0"
        body = b"FVER" + struct.pack(">QI", 4, 0x01050000)
        body += b"PROP" + struct.pack(">Q", len(prop)) + prop
        body += b"DSD " + struct.pack(">Q", n * ch)
        with open(dst, "wb") as o:
            o.write(b"FRM8" + struct.pack(">Q", 4 + len(body) + n * ch) + b"DSD " + body)
            for chans in dsf_blocks(f, ch, n):
                out = bytearray(len(chans[0]) * ch)
                for c, d in enumerate(chans):
                    out[c::ch] = d
                o.write(out)


def read_dff(f):
    head = f.read(1 << 16)
    if head[:4] != b"FRM8":
        sys.exit("not a DFF file")
    pos, ch, fs = 16, 0, 0
    while pos + 12 <= len(head):
        cid, size = head[pos : pos + 4], struct.unpack_from(">Q", head, pos + 4)[0]
        if cid == b"PROP":
            q = pos + 16
            while q + 12 <= pos + 12 + size:
                sid, s = head[q : q + 4], struct.unpack_from(">Q", head, q + 4)[0]
                if sid == b"FS  ":
                    fs = struct.unpack_from(">I", head, q + 12)[0]
                if sid == b"CHNL":
                    ch = struct.unpack_from(">H", head, q + 12)[0]
                if sid == b"CMPR" and head[q + 12 : q + 16] != b"DSD ":
                    sys.exit("DFF: compressed")
                q += 12 + s + (s & 1)
        if cid == b"DSD ":
            if fs != 2822400:
                sys.exit("DFF: need DSD64")
            return ch, pos + 12, size // ch
        pos += 12 + size + (size & 1)
    sys.exit("DFF: no DSD chunk")


def dff2dsf(src, dst):
    with open(src, "rb") as f:
        ch, off, n = read_dff(f)
        groups = -(-n // BLOCK)
        data = groups * BLOCK * ch
        with open(dst, "wb") as o:
            o.write(b"DSD " + struct.pack("<QQQ", 28, 92 + data, 0))
            o.write(b"fmt " + struct.pack("<QIIIIIIQII", 52, 1, 0, DSF_TYPE[ch], ch, 2822400, 1, n * 8, BLOCK, 0))
            o.write(b"data" + struct.pack("<Q", 12 + data))
            f.seek(off)
            done = 0
            while done < n:
                take = min(BLOCK, n - done)
                inter = f.read(take * ch)
                for c in range(ch):
                    o.write(inter[c::ch].translate(REV).ljust(BLOCK, b"\0"))
                done += take


def odd(src, dst, drop, idle_bytes):
    if not 1 <= drop <= 7:
        sys.exit("DROP: 1 to 7")
    with open(src, "rb") as f:
        ch, samples = read_dsf(f)
        if samples % 8:
            sys.exit("odd: the input's sample count is already not a multiple of 8")
        data = bytearray(f.read())  # header was read; the rest is the data chunk body
    n = samples // 8
    new = samples - drop
    head = bytearray(open(src, "rb").read(92))
    struct.pack_into("<Q", head, 64, new)

    def at(c, i):  # offset in `data` of byte i of channel c
        return (i // BLOCK) * BLOCK * ch + c * BLOCK + i % BLOCK

    idle = REV[0x69]  # 0x69 as stored LSB first
    for c in range(ch):
        for i in range(max(0, n - idle_bytes), n):
            data[at(c, i)] = idle
        # The last byte keeps 8 - DROP samples (low bits, LSB first); its top DROP bits
        # are padding: zero them.
        data[at(c, n - 1)] &= (1 << (8 - drop)) - 1
    with open(dst, "wb") as o:
        o.write(head)
        o.write(data)


def head(src, dst, seconds):
    with open(src, "rb") as f:
        ch, samples = read_dsf(f)
        n = min(samples // 8, int(seconds * 2822400) // 8)
        groups = -(-n // BLOCK)
        data = groups * BLOCK * ch
        with open(dst, "wb") as o:
            o.write(b"DSD " + struct.pack("<QQQ", 28, 92 + data, 0))
            o.write(b"fmt " + struct.pack("<QIIIIIIQII", 52, 1, 0, DSF_TYPE[ch], ch, 2822400, 1, n * 8, BLOCK, 0))
            o.write(b"data" + struct.pack("<Q", 12 + data))
            for chans in dsf_blocks(f, ch, n):
                for d in chans:
                    o.write(d.translate(REV).ljust(BLOCK, b"\0"))


def main():
    a = sys.argv[1:]
    if len(a) == 3 and a[0] == "dsf2dff":
        dsf2dff(a[1], a[2])
    elif len(a) == 3 and a[0] == "dff2dsf":
        dff2dsf(a[1], a[2])
    elif len(a) in (4, 5) and a[0] == "odd":
        odd(a[1], a[2], int(a[3]), int(a[4]) if len(a) == 5 else 0)
    elif len(a) == 4 and a[0] == "head":
        head(a[1], a[2], float(a[3]))
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
