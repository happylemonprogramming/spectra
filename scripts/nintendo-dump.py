#!/usr/bin/env python3
"""Read GameCube and Wii discs on an ordinary LG DVD drive, without OmniDrive.

A spike for Phase 3. The drive refuses to return a Nintendo disc's sectors,
since their scrambling is not the DVD standard's and the EDC fails, but it has
already read them into its cache by then. READ BUFFER (mode 1) hands the cache
back as raw 2064-byte frames: ID, IED, CPR_MAI, 2048 bytes and EDC. The drive
has descrambled them with the standard seed, which is an XOR, so undoing it
gives the frame as pressed. Each 16-sector block of a Nintendo disc has its own
15-bit seed, and since the scrambler and the EDC are both linear, the seed
whose keystream makes the EDC check is a table lookup rather than a search.

Found on an HL-DT-ST DVDRAM GUD0N (LW01) behind an Initio INIC-1618L bridge:
one READ at sector x fills a ring of 144 frames with x onward, READ BUFFER
returns at most 0xDBB0 bytes (27 frames) at a time, and frame n of the ring is
at offset n * 0x810.

    nintendo-dump.py probe [--drive /dev/sg0]
    nintendo-dump.py dump OUT.iso [--sectors N] [--start N] [--standard]

--standard reads an ordinary DVD the same way, to test the drive and the loop
without a Nintendo disc.
"""
import argparse, ctypes, fcntl, os, struct, sys, time

FRAME = 0x810            # 2064 bytes: a raw DVD frame
WINDOW = 27              # whole frames per READ BUFFER, which caps at 0xDBB0 bytes
RING = 144               # frames one READ leaves in the drive's cache
PSN0 = 0x30000           # physical sector number of LBA 0

GAMECUBE_SECTORS = 712_880
WII_SECTORS = 2_294_912
WII_DL_SECTORS = 4_155_840


# --- SG_IO -------------------------------------------------------------------

class _SgIoHdr(ctypes.Structure):
    _fields_ = [
        ("interface_id", ctypes.c_int), ("dxfer_direction", ctypes.c_int),
        ("cmd_len", ctypes.c_ubyte), ("mx_sb_len", ctypes.c_ubyte),
        ("iovec_count", ctypes.c_ushort), ("dxfer_len", ctypes.c_uint),
        ("dxferp", ctypes.c_void_p), ("cmdp", ctypes.c_void_p),
        ("sbp", ctypes.c_void_p), ("timeout", ctypes.c_uint),
        ("flags", ctypes.c_uint), ("pack_id", ctypes.c_int),
        ("usr_ptr", ctypes.c_void_p), ("status", ctypes.c_ubyte),
        ("masked_status", ctypes.c_ubyte), ("msg_status", ctypes.c_ubyte),
        ("sb_len_wr", ctypes.c_ubyte), ("host_status", ctypes.c_ushort),
        ("driver_status", ctypes.c_ushort), ("resid", ctypes.c_int),
        ("duration", ctypes.c_uint), ("info", ctypes.c_uint),
    ]


class Drive:
    def __init__(self, path):
        self.fd = os.open(path, os.O_RDWR | os.O_NONBLOCK)

    def cmd(self, cdb, length=0, out=None, timeout=30000):
        """Send a CDB; returns (data, sense) where sense is (key, asc, ascq) or None."""
        cdb = bytes(cdb)
        size = len(out) if out is not None else length
        buf = ctypes.create_string_buffer(out if out is not None else b"", max(size, 1))
        cbuf = ctypes.create_string_buffer(cdb, len(cdb))
        sense = ctypes.create_string_buffer(64)
        h = _SgIoHdr(interface_id=ord("S"), cmd_len=len(cdb), mx_sb_len=64,
                     dxfer_len=size, timeout=timeout)
        h.dxfer_direction = -2 if out is not None else (-3 if length else -1)
        h.dxferp = ctypes.cast(buf, ctypes.c_void_p)
        h.cmdp = ctypes.cast(cbuf, ctypes.c_void_p)
        h.sbp = ctypes.cast(sense, ctypes.c_void_p)
        fcntl.ioctl(self.fd, 0x2285, h)
        sk = None
        if h.status or h.host_status or h.driver_status:
            s = sense.raw[:h.sb_len_wr]
            sk = (s[2] & 0xF, s[12], s[13]) if len(s) > 13 else ("status", h.status, h.host_status)
        return (buf.raw[:size - h.resid] if length else b""), sk

    def inquiry(self):
        d, _ = self.cmd([0x12, 0, 0, 0, 36, 0], 36)
        return " ".join(d[8:36].decode("ascii", "replace").split())

    def ready(self):
        return self.cmd([0, 0, 0, 0, 0, 0])[1]

    def capacity(self):
        d, sk = self.cmd([0x25] + [0] * 9, 8)
        return (int.from_bytes(d[:4], "big") + 1) if not sk else None

    def physical_format(self):
        d, sk = self.cmd([0xAD, 0, 0, 0, 0, 0, 0, 0, 0x08, 0x04, 0, 0], 0x804)
        return (d[4:8] if not sk else None), sk

    def read(self, lba, count=1):
        return self.cmd(struct.pack(">BBIIBB", 0xA8, 0, lba, count, 0, 0), 2048 * count)

    def ring(self):
        """Every frame the last READ left in the cache."""
        w = b""
        for slot in range(0, RING, WINDOW):
            n = min(WINDOW, RING - slot)
            o, l = (slot * FRAME).to_bytes(3, "big"), (n * FRAME).to_bytes(3, "big")
            d, sk = self.cmd(bytes([0x3C, 0x01, 0]) + o + l + b"\0", n * FRAME)
            if sk:
                break
            w += d
        return [w[i:i + FRAME] for i in range(0, len(w) - FRAME + 1, FRAME)]

    def retries(self, count=None):
        """Read the read-retry count from mode page 1, or set it (volatile)."""
        d, sk = self.cmd([0x5A, 0, 0x01, 0, 0, 0, 0, 0, 20, 0], 20)
        if sk or len(d) < 12:
            return None
        if count is None:
            return d[8 + 3]
        page = bytearray(d)
        page[0:2] = b"\0\0"
        page[8] &= 0x3F
        page[8 + 3] = count
        self.cmd([0x55, 0x10, 0, 0, 0, 0, 0, 0, len(page), 0], out=bytes(page))
        return count


def psn(frame):
    return int.from_bytes(frame[1:4], "big")


# --- ECMA-267 scrambling and EDC ---------------------------------------------

ECMA_IVS = [0x0001, 0x5500, 0x0002, 0x2A00, 0x0004, 0x5400, 0x0008, 0x2800,
            0x0010, 0x5000, 0x0020, 0x2001, 0x0040, 0x4002, 0x0080, 0x0005]

_EDC = []
for _i in range(256):
    _r = _i << 24
    for _ in range(8):
        _r = ((_r << 1) ^ 0x80000011) & 0xFFFFFFFF if _r & 0x80000000 else (_r << 1) & 0xFFFFFFFF
    _EDC.append(_r)


def edc(data):
    e = 0
    for b in data:
        e = _EDC[((e >> 24) ^ b) & 0xFF] ^ ((e << 8) & 0xFFFFFFFF)
    return e


def edc_ok(frame):
    return edc(frame[:2060]) == int.from_bytes(frame[2060:2064], "big")


_KS = {}


def keystream(seed):
    ks = _KS.get(seed)
    if ks is None:
        lfsr, out = seed, 0
        for _ in range(2048 * 8):
            bit = lfsr >> 14
            lfsr = ((lfsr << 1) | (bit ^ ((lfsr >> 10) & 1))) & 0x7FFF
            out = (out << 1) | bit
        ks = _KS[seed] = out
    return ks


def xor_main(frame, seed):
    """XOR a frame's 2048 bytes of main data (12..2059) with a seed's keystream."""
    main = int.from_bytes(frame[12:2060], "big") ^ keystream(seed)
    return frame[:12] + main.to_bytes(2048, "big") + frame[2060:]


def standard_seed(p):
    return ECMA_IVS[(p >> 4) & 0xF]


_SEED_BY_EDC = None


def solve_seed(scrambled):
    """The seed that makes this frame's EDC check, or None."""
    global _SEED_BY_EDC
    if _SEED_BY_EDC is None:
        basis = [edc(bytes(12) + keystream(1 << i).to_bytes(2048, "big")) for i in range(15)]
        _SEED_BY_EDC, e, prev = {}, 0, 0
        for n in range(1, 0x8000):
            g = n ^ (n >> 1)
            e ^= basis[(g ^ prev).bit_length() - 1]
            prev = g
            _SEED_BY_EDC.setdefault(e, g)
    target = edc(scrambled[:2060]) ^ int.from_bytes(scrambled[2060:2064], "big")
    return _SEED_BY_EDC.get(target)


def nintendo_data(frame):
    """2048 bytes of a Nintendo frame from the cache, and its seed; (None, None) if bad.

    Nintendo's 2048 bytes start at byte 6, where a DVD's CPR_MAI would be."""
    for f in (xor_main(frame, standard_seed(psn(frame))), frame):
        s = solve_seed(f)
        if s is not None:
            plain = xor_main(f, s)
            if edc_ok(plain):
                return plain[6:2054], s
    return None, None


def standard_data(frame):
    return (frame[12:2060], None) if edc_ok(frame) else (None, None)


def describe(header):
    gid = header[:6].decode("ascii", "replace")
    title = header[0x20:0x60].split(b"\0")[0].decode("ascii", "replace").strip()
    if header[0x1C:0x20] == bytes.fromhex("c2339f3d"):
        return "GameCube", gid, title
    if header[0x18:0x1C] == bytes.fromhex("5d1c9ea3"):
        return "Wii", gid, title
    return None, gid, title


# --- Harvesting --------------------------------------------------------------

def harvest(d, start, end, decode, on_sector, progress=None):
    """Read sectors start..end, calling on_sector(n, data or None) in any order."""
    done, x = set(), start
    stats = dict(reads=0, flushes=0, bad=0, seeds=set())
    while x < end:
        for attempt in range(4):
            if attempt:
                # A read far away empties the ring, so the next read at x
                # refills it from x.
                d.read(x + 0x8000 if x + 0x8000 < end else max(0, x - 0x8000))
                stats["flushes"] += 1
            sense = d.read(x)[1]
            stats["reads"] += 1
            for f in d.ring():
                n = psn(f) - PSN0
                if start <= n < end and n not in done:
                    data, seed = decode(f)
                    if data is not None:
                        done.add(n)
                        stats["seeds"].add(seed)
                        on_sector(n, data)
            if x in done:
                break
        else:
            print(f"\n  sector {x}: no good frame (sense {sense})", file=sys.stderr)
            done.add(x)
            stats["bad"] += 1
            on_sector(x, None)
        while x < end and x in done:
            x += 1
        if progress:
            progress(x, stats)
    return stats


# --- Commands ----------------------------------------------------------------

def probe(d):
    print("Drive:   ", d.inquiry())
    print("Ready:   ", d.ready() or "yes")
    print("Capacity:", d.capacity())
    pf, sk = d.physical_format()
    print("Physical format:", pf.hex() if pf else f"refused {sk}")
    print("Read retries:", d.retries())

    t = time.time()
    data, sense = d.read(0)
    print(f"READ LBA 0: {len(data)} bytes, sense {sense}, {(time.time() - t) * 1000:.0f} ms")
    frames = d.ring()
    print(f"Cache: {len(frames)} frames, PSN {psn(frames[0]):#x}..{psn(frames[-1]):#x}" if frames else "Cache: empty")

    std = sum(1 for f in frames if edc_ok(f))
    # A standard DVD's frames solve too, to the standard seeds; only another
    # seed means a Nintendo disc.
    nin = [(psn(f), *nintendo_data(f)) for f in frames]
    good = [x for x in nin if x[1] is not None and x[2] != standard_seed(x[0])]
    print(f"Frames that check as a standard DVD: {std}/{len(frames)}")
    print(f"Frames with Nintendo seeds:          {len(good)}/{len(frames)}"
          + (f", seeds {sorted({s for _, _, s in good})[:8]}" if good else ""))

    first = next((x for x in good if x[0] == PSN0), None)
    if first:
        system, gid, title = describe(first[1])
        print(f"\n{system or 'Unknown'} disc: {gid} {title}")
        print("This drive can read it: run `nintendo-dump.py dump OUT.iso`.")
    elif std and data:
        print("\nAn ordinary DVD: try `dump --standard` to test the loop.")
    else:
        print("\nNo Nintendo frames found. Save this output: it says how the drive refuses.")


def dump(d, out, start, sectors, standard):
    decode = standard_data if standard else nintendo_data
    if sectors is None:
        if standard:
            sectors = d.capacity()
        else:
            d.read(0)
            first = next((nintendo_data(f)[0] for f in d.ring() if psn(f) == PSN0), None)
            system = describe(first)[0] if first else None
            if system == "GameCube":
                sectors = GAMECUBE_SECTORS
            elif system == "Wii":
                d.read(WII_SECTORS)
                dual = any(nintendo_data(f)[0] for f in d.ring())
                sectors = WII_DL_SECTORS if dual else WII_SECTORS
            else:
                sys.exit("Not a GameCube or Wii disc as far as the cache shows; try `probe`.")
            print(f"{system} disc, {sectors} sectors", file=sys.stderr)

    old_retries = d.retries()
    if not standard and old_retries:
        d.retries(0)      # a Nintendo sector always fails; do not let the drive retry it
    t0, end = time.time(), start + sectors
    shown = [0.0]

    def progress(x, st):
        if time.time() - shown[0] < 1 and x < end:
            return
        shown[0] = time.time()
        done = x - start
        rate = done * 2048 / max(time.time() - t0, 1e-6) / 1e6
        eta = (end - x) * 2048 / max(rate, 1e-6) / 1e6
        print(f"\r{done}/{sectors} sectors  {rate:.2f} MB/s  {eta / 60:.0f} min left  bad {st['bad']}   ",
              end="", file=sys.stderr, flush=True)

    try:
        with open(out, "wb") as iso:
            iso.truncate(sectors * 2048)

            def put(n, data):
                iso.seek((n - start) * 2048)
                iso.write(data if data is not None else bytes(2048))

            st = harvest(d, start, end, decode, put, progress)
    finally:
        if not standard and old_retries:
            d.retries(old_retries)
    dt = time.time() - t0
    print(f"\n{sectors} sectors in {dt:.0f} s ({sectors * 2048 / dt / 1e6:.2f} MB/s), "
          f"{st['reads']} reads, {st['flushes']} flushes, {st['bad']} bad", file=sys.stderr)
    return st["bad"] == 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--drive", default="/dev/sg0")
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("probe")
    p = sub.add_parser("dump")
    p.add_argument("out")
    p.add_argument("--start", type=lambda s: int(s, 0), default=0)
    p.add_argument("--sectors", type=lambda s: int(s, 0))
    p.add_argument("--standard", action="store_true")
    a = ap.parse_args()
    d = Drive(a.drive)
    if a.cmd == "probe":
        probe(d)
    else:
        sys.exit(0 if dump(d, a.out, a.start, a.sectors, a.standard) else 1)


if __name__ == "__main__":
    main()
