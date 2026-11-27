use std::{
    fs,
    io::{self, BufWriter, Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const fn crc_tables() -> [[u32; 256]; 8] {
    let mut t = [[0u32; 256]; 8];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        t[0][i] = c;
        i += 1;
    }
    let mut i = 0;
    while i < 256 {
        let mut s = 1;
        while s < 8 {
            t[s][i] = (t[s - 1][i] >> 8) ^ t[0][(t[s - 1][i] & 0xFF) as usize];
            s += 1;
        }
        i += 1;
    }
    t
}
static CRC: [[u32; 256]; 8] = crc_tables();

pub fn crc32(crc: u32, data: &[u8]) -> u32 {
    let mut c = !crc;
    let mut ch = data.chunks_exact(8);
    for b in &mut ch {
        let lo = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) ^ c;
        let hi = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
        c = CRC[7][(lo & 0xFF) as usize]
            ^ CRC[6][((lo >> 8) & 0xFF) as usize]
            ^ CRC[5][((lo >> 16) & 0xFF) as usize]
            ^ CRC[4][(lo >> 24) as usize]
            ^ CRC[3][(hi & 0xFF) as usize]
            ^ CRC[2][((hi >> 8) & 0xFF) as usize]
            ^ CRC[1][((hi >> 16) & 0xFF) as usize]
            ^ CRC[0][(hi >> 24) as usize];
    }
    for &b in ch.remainder() {
        c = CRC[0][((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    !c
}

const LBASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEXT: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DBASE: [u16; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DEXT: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
const CL_ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

const fn len_codes() -> [u8; 259] {
    let mut t = [0u8; 259];
    let mut c = 0;
    while c < 29 {
        let mut l = LBASE[c] as usize;
        let end = if c == 28 { 259 } else { LBASE[c + 1] as usize };
        while l < end {
            t[l] = c as u8;
            l += 1;
        }
        c += 1;
    }
    t[258] = 28;
    t
}
static LEN_CODE: [u8; 259] = len_codes();

fn dist_code(d: usize) -> usize {
    let x = d - 1;
    if x < 4 {
        x
    } else {
        let l = (usize::BITS - 1 - x.leading_zeros()) as usize;
        2 * l + ((x >> (l - 1)) & 1)
    }
}

fn rev_bits(mut c: u32, n: u32) -> u32 {
    let mut r = 0;
    for _ in 0..n {
        r = (r << 1) | (c & 1);
        c >>= 1;
    }
    r
}

fn code_lengths(freq: &[u32], limit: u32, len: &mut [u8]) {
    let n = freq.len();
    len[..n].fill(0);
    let mut used = [0usize; 286];
    let mut ulen = 0usize;
    for (i, &f) in freq.iter().enumerate() {
        if f > 0 {
            used[ulen] = i;
            ulen += 1;
        }
    }
    match ulen {
        0 => return,
        1 => {
            len[used[0]] = 1;
            return;
        }
        _ => {}
    }
    let used = &mut used[..ulen];
    let mut parent = [usize::MAX; 572];
    let mut heap = std::collections::BinaryHeap::with_capacity(ulen);
    for &i in used.iter() {
        heap.push(std::cmp::Reverse((freq[i] as u64, i)));
    }
    let mut next = n;
    while heap.len() > 1 {
        let std::cmp::Reverse((fa, a)) = heap.pop().unwrap();
        let std::cmp::Reverse((fb, b)) = heap.pop().unwrap();
        parent[a] = next;
        parent[b] = next;
        heap.push(std::cmp::Reverse((fa + fb, next)));
        next += 1;
    }
    for &i in used.iter() {
        let (mut d, mut p) = (0u32, i);
        while parent[p] != usize::MAX {
            p = parent[p];
            d += 1;
        }
        len[i] = d.min(limit) as u8;
    }
    let unit = |l: u8| 1u64 << (limit - l as u32);
    let mut kraft: u64 = used.iter().map(|&i| unit(len[i])).sum();
    let full = 1u64 << limit;
    used.sort_by_key(|&i| std::cmp::Reverse(freq[i]));
    while kraft > full {
        let i = *used.iter().filter(|&&i| (len[i] as u32) < limit).max_by_key(|&&i| (len[i], std::cmp::Reverse(freq[i]))).unwrap();
        kraft -= unit(len[i]) / 2;
        len[i] += 1;
    }
    loop {
        let mut changed = false;
        for &i in used.iter() {
            if kraft < full && len[i] > 1 && full - kraft >= unit(len[i]) {
                kraft += unit(len[i]);
                len[i] -= 1;
                changed = true;
            }
        }
        if kraft == full || !changed {
            break;
        }
    }
}

fn codes(len: &[u8], out: &mut [u32]) {
    let mut count = [0u32; 16];
    for &l in len {
        count[l as usize] += 1;
    }
    count[0] = 0;
    let (mut next, mut code) = ([0u32; 16], 0u32);
    for b in 1..16 {
        code = (code + count[b - 1]) << 1;
        next[b] = code;
    }
    for (i, &l) in len.iter().enumerate() {
        if l == 0 {
            out[i] = 0;
        } else {
            let c = next[l as usize];
            next[l as usize] += 1;
            out[i] = rev_bits(c, l as u32);
        }
    }
}

struct BitW {
    acc: u64,
    n: u32,
}

impl BitW {
    fn put(&mut self, out: &mut Vec<u8>, v: u32, bits: u32) {
        self.acc |= (v as u64) << self.n;
        self.n += bits;
        while self.n >= 8 {
            out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }
    fn align(&mut self, out: &mut Vec<u8>) {
        if self.n > 0 {
            out.push(self.acc as u8);
            self.acc = 0;
            self.n = 0;
        }
    }
}

const WIN: usize = 32768;
const HASH_BITS: u32 = 15;
const MAX_CHAIN: u32 = 16;
const LAZY_BELOW: u32 = 16;

pub struct Deflater {
    bw: BitW,
    head: Vec<u32>,
    prev: Vec<u32>,
    next_ins: usize,
    hist: Vec<u8>,
    buf: Vec<u8>,
    syms: Vec<u32>,
}

fn looks_random(d: &[u8]) -> bool {
    if d.len() < 4096 {
        return false;
    }
    let mut count = [0u32; 256];
    let step = d.len() / 8;
    for k in 0..8 {
        for &b in &d[k * step..k * step + 512] {
            count[b as usize] += 1;
        }
    }
    let h: f64 = count.iter().filter(|&&c| c > 0).map(|&c| { let p = c as f64 / 4096.0; -p * p.log2() }).sum();
    h > 7.8
}

impl Deflater {
    pub fn new() -> Self {
        Deflater { bw: BitW { acc: 0, n: 0 }, head: vec![u32::MAX; 1 << HASH_BITS], prev: vec![u32::MAX; WIN], next_ins: 0, hist: Vec::new(), buf: Vec::new(), syms: Vec::new() }
    }

    fn hash(d: &[u8], i: usize) -> usize {
        let v = u32::from_le_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]]);
        (v.wrapping_mul(0x9E37_79B1) >> (32 - HASH_BITS)) as usize
    }

    fn ins(&mut self, d: &[u8], i: usize) {
        if i >= self.next_ins && i + 4 <= d.len() {
            let h = Self::hash(d, i);
            self.prev[i & (WIN - 1)] = self.head[h];
            self.head[h] = i as u32;
            self.next_ins = i + 1;
        }
    }

    fn search(&self, d: &[u8], i: usize) -> (u32, u32) {
        if i + 4 > d.len() {
            return (0, 0);
        }
        let max = (d.len() - i).min(258);
        let (mut best, mut best_d) = (0usize, 0usize);
        let mut cand = self.head[Self::hash(d, i)];
        let mut chain = MAX_CHAIN;
        while cand != u32::MAX && chain > 0 {
            let c = cand as usize;
            if c >= i || i - c > WIN {
                break;
            }
            let mut l = 0;
            while l + 8 <= max {
                let x = u64::from_le_bytes(d[c + l..c + l + 8].try_into().unwrap()) ^ u64::from_le_bytes(d[i + l..i + l + 8].try_into().unwrap());
                if x != 0 {
                    l += (x.trailing_zeros() / 8) as usize;
                    break;
                }
                l += 8;
            }
            while l < max && d[c + l] == d[i + l] {
                l += 1;
            }
            if l > best {
                best = l;
                best_d = i - c;
                if l == max {
                    break;
                }
                if l >= 128 {
                    chain >>= 2;
                }
            }
            let nxt = self.prev[c & (WIN - 1)];
            if nxt >= cand {
                break;
            }
            cand = nxt;
            chain -= 1;
        }
        if best < 4 && !(best == 3 && best_d <= 4096) { (0, 0) } else { (best as u32, best_d as u32) }
    }

    pub fn chunk(&mut self, data: &[u8], out: &mut Vec<u8>) {
        if data.is_empty() {
            return;
        }
        if looks_random(data) {
            self.stored(data, out);
        } else {
            self.matched(data, out);
        }
        if data.len() >= WIN {
            self.hist.clear();
            self.hist.extend_from_slice(&data[data.len() - WIN..]);
        } else {
            self.hist.extend_from_slice(data);
            if self.hist.len() > WIN {
                self.hist.drain(..self.hist.len() - WIN);
            }
        }
    }

    fn matched(&mut self, data: &[u8], out: &mut Vec<u8>) {
        let h = self.hist.len();
        let mut buf = std::mem::take(&mut self.buf);
        buf.clear();
        buf.reserve(h + data.len());
        buf.extend_from_slice(&self.hist);
        buf.extend_from_slice(data);
        let mut syms = std::mem::take(&mut self.syms);
        syms.clear();
        syms.reserve(data.len() / 2 + 16);
        let d = &buf[..];
        self.head.fill(u32::MAX);
        self.next_ins = 0;
        for j in 0..h {
            self.ins(d, j);
        }
        let n = d.len();
        let mut i = h;
        while i < n {
            let mut m = self.search(d, i);
            self.ins(d, i);
            if m.0 >= 3 {
                while m.0 < LAZY_BELOW && i + 1 < n {
                    let m2 = self.search(d, i + 1);
                    if m2.0 <= m.0 {
                        break;
                    }
                    syms.push(d[i] as u32);
                    i += 1;
                    self.ins(d, i);
                    m = m2;
                }
                syms.push(m.0 << 16 | m.1);
                let end = i + m.0 as usize;
                for j in (i + 1..end).filter(|&j| m.0 < 48 || j < i + 17 || j + 16 >= end) {
                    self.ins(d, j);
                }
                i = end;
            } else {
                syms.push(d[i] as u32);
                i += 1;
            }
        }
        self.block(data, &syms, out);
        self.buf = buf;
        self.syms = syms;
    }

    fn stored(&mut self, raw: &[u8], out: &mut Vec<u8>) {
        for part in raw.chunks(65535) {
            self.bw.put(out, 0, 3);
            self.bw.align(out);
            out.extend_from_slice(&(part.len() as u16).to_le_bytes());
            out.extend_from_slice(&(!(part.len() as u16)).to_le_bytes());
            out.extend_from_slice(part);
        }
    }

    fn block(&mut self, raw: &[u8], syms: &[u32], out: &mut Vec<u8>) {
        let (mut lf, mut df) = ([0u32; 286], [0u32; 30]);
        lf[256] = 1;
        for &s in syms {
            if s >> 16 == 0 {
                lf[s as usize] += 1;
            } else {
                lf[257 + LEN_CODE[(s >> 16) as usize] as usize] += 1;
                df[dist_code((s & 0xFFFF) as usize)] += 1;
            }
        }
        if df.iter().all(|&f| f == 0) {
            df[0] = 1;
        }
        let (mut ll, mut dl) = ([0u8; 286], [0u8; 30]);
        code_lengths(&lf, 15, &mut ll);
        code_lengths(&df, 15, &mut dl);
        let hlit = (257..=286).rev().find(|&n| ll[n - 1] != 0).unwrap_or(257);
        let hdist = (1..=30).rev().find(|&n| dl[n - 1] != 0).unwrap_or(1);
        let mut all = [0u8; 316];
        all[..hlit].copy_from_slice(&ll[..hlit]);
        all[hlit..hlit + hdist].copy_from_slice(&dl[..hdist]);
        let all_len = hlit + hdist;
        let mut rl = [(0u8, 0u8); 316];
        let mut rlen = 0usize;
        let mut k = 0;
        while k < all_len {
            let v = all[k];
            let mut run = 1;
            while k + run < all_len && all[k + run] == v {
                run += 1;
            }
            k += run;
            if v == 0 {
                while run >= 3 {
                    let r = run.min(138);
                    rl[rlen] = if r >= 11 { (18, (r - 11) as u8) } else { (17, (r - 3) as u8) };
                    rlen += 1;
                    run -= r;
                }
            } else {
                rl[rlen] = (v, 0);
                rlen += 1;
                run -= 1;
                while run >= 3 {
                    let r = run.min(6);
                    rl[rlen] = (16, (r - 3) as u8);
                    rlen += 1;
                    run -= r;
                }
            }
            for _ in 0..run {
                rl[rlen] = (v, 0);
                rlen += 1;
            }
        }
        let rl = &rl[..rlen];
        let mut cf = [0u32; 19];
        for &(s, _) in rl {
            cf[s as usize] += 1;
        }
        let mut cl = [0u8; 19];
        code_lengths(&cf, 7, &mut cl);
        let hclen = (4..=19).rev().find(|&n| cl[CL_ORDER[n - 1]] != 0).unwrap_or(4);
        let mut bits: u64 = 17 + 3 * hclen as u64;
        bits += rl.iter().map(|&(s, _)| cl[s as usize] as u64 + [0, 0, 0, 2, 3, 7][if s >= 16 { s as usize - 13 } else { 0 }]).sum::<u64>();
        bits += (0..286).map(|i| lf[i] as u64 * (ll[i] as u64 + if i >= 257 { LEXT[i - 257] as u64 } else { 0 })).sum::<u64>();
        bits += (0..30).map(|i| df[i] as u64 * (dl[i] as u64 + DEXT[i] as u64)).sum::<u64>();
        let stored_bits = raw.len() as u64 * 8 + 40 * (raw.len() as u64 / 65535 + 1);
        if bits >= stored_bits {
            self.stored(raw, out);
            return;
        }
        let (mut lc, mut dc, mut cc) = ([0u32; 286], [0u32; 30], [0u32; 19]);
        codes(&ll, &mut lc);
        codes(&dl, &mut dc);
        codes(&cl, &mut cc);
        let bw = &mut self.bw;
        bw.put(out, 0b100, 3);
        bw.put(out, (hlit - 257) as u32, 5);
        bw.put(out, (hdist - 1) as u32, 5);
        bw.put(out, (hclen - 4) as u32, 4);
        for &o in &CL_ORDER[..hclen] {
            bw.put(out, cl[o] as u32, 3);
        }
        for &(s, x) in rl {
            bw.put(out, cc[s as usize], cl[s as usize] as u32);
            match s {
                16 => bw.put(out, x as u32, 2),
                17 => bw.put(out, x as u32, 3),
                18 => bw.put(out, x as u32, 7),
                _ => {}
            }
        }
        for &s in syms {
            if s >> 16 == 0 {
                bw.put(out, lc[s as usize], ll[s as usize] as u32);
            } else {
                let (len, dist) = ((s >> 16) as usize, (s & 0xFFFF) as usize);
                let c = LEN_CODE[len] as usize;
                bw.put(out, lc[257 + c], ll[257 + c] as u32);
                bw.put(out, (len - LBASE[c] as usize) as u32, LEXT[c] as u32);
                let d = dist_code(dist);
                bw.put(out, dc[d], dl[d] as u32);
                bw.put(out, (dist - DBASE[d] as usize) as u32, DEXT[d] as u32);
            }
        }
        bw.put(out, lc[256], ll[256] as u32);
    }

    pub fn finish(&mut self, out: &mut Vec<u8>) {
        self.bw.put(out, 0b011, 3);
        self.bw.put(out, 0, 7);
        self.bw.align(out);
    }
}

pub fn deflate(data: &[u8]) -> Vec<u8> {
    let mut d = Deflater::new();
    let mut out = Vec::with_capacity(data.len() / 2 + 64);
    for c in data.chunks(CHUNK) {
        d.chunk(c, &mut out);
    }
    d.finish(&mut out);
    out
}

pub const CHUNK: usize = 1 << 20;
const FLUSH_AT: usize = (1 << 19) + WIN;

struct Bits<R: Read> {
    r: R,
    buf: Box<[u8; 1 << 16]>,
    pos: usize,
    len: usize,
    acc: u64,
    n: u32,
}

fn eof() -> io::Error {
    io::Error::new(io::ErrorKind::UnexpectedEof, "compressed data ends too early")
}

fn bad(m: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, m.to_string())
}

impl<R: Read> Bits<R> {
    fn new(r: R) -> Self {
        Bits { r, buf: Box::new([0; 1 << 16]), pos: 0, len: 0, acc: 0, n: 0 }
    }
    fn fill_buf(&mut self) -> io::Result<()> {
        if self.pos == self.len {
            self.len = self.r.read(&mut self.buf[..])?;
            self.pos = 0;
        }
        Ok(())
    }
    fn refill(&mut self) -> io::Result<()> {
        while self.n <= 56 {
            self.fill_buf()?;
            if self.pos == self.len {
                break;
            }
            self.acc |= (self.buf[self.pos] as u64) << self.n;
            self.pos += 1;
            self.n += 8;
        }
        Ok(())
    }
    fn peek(&mut self, k: u32) -> io::Result<u32> {
        if self.n < k {
            self.refill()?;
        }
        Ok((self.acc & ((1u64 << k) - 1)) as u32)
    }
    fn take(&mut self, k: u32) -> io::Result<()> {
        if self.n < k {
            return Err(eof());
        }
        self.acc >>= k;
        self.n -= k;
        Ok(())
    }
    fn bits(&mut self, k: u32) -> io::Result<u32> {
        let v = self.peek(k)?;
        self.take(k)?;
        Ok(v)
    }
    fn bytes(&mut self, mut n: usize, out: &mut Vec<u8>) -> io::Result<()> {
        while n > 0 && self.n >= 8 {
            out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
            n -= 1;
        }
        while n > 0 {
            self.fill_buf()?;
            if self.pos == self.len {
                return Err(eof());
            }
            let k = n.min(self.len - self.pos);
            out.extend_from_slice(&self.buf[self.pos..self.pos + k]);
            self.pos += k;
            n -= k;
        }
        Ok(())
    }
}

struct Table {
    max: u32,
    t: Vec<u16>,
}

impl Table {
    fn new(len: &[u8]) -> io::Result<Table> {
        let max = len.iter().copied().max().unwrap_or(0) as u32;
        let mut count = [0u32; 16];
        for &l in len {
            count[l as usize] += 1;
        }
        count[0] = 0;
        let (mut next, mut code) = ([0u32; 16], 0u32);
        for b in 1..16 {
            code = (code + count[b - 1]) << 1;
            next[b] = code;
            if count[b] > 0 && code + count[b] > 1 << b {
                return Err(bad("oversubscribed Huffman code"));
            }
        }
        let mut t = vec![0u16; 1 << max];
        for (s, &l) in len.iter().enumerate() {
            if l == 0 {
                continue;
            }
            let c = rev_bits(next[l as usize], l as u32) as usize;
            next[l as usize] += 1;
            for k in (c..t.len()).step_by(1 << l) {
                t[k] = (l as u16) << 9 | s as u16;
            }
        }
        Ok(Table { max, t })
    }
    fn sym<R: Read>(&self, b: &mut Bits<R>) -> io::Result<usize> {
        let e = *self.t.get(b.peek(self.max)? as usize).ok_or_else(|| bad("bad Huffman code"))?;
        if e == 0 {
            return Err(bad("bad Huffman code"));
        }
        b.take((e >> 9) as u32)?;
        Ok((e & 511) as usize)
    }
}

pub fn inflate<R: Read, W: Write>(r: R, w: &mut W, progress: &mut dyn FnMut(u64) -> bool) -> io::Result<(u32, u64)> {
    let mut b = Bits::new(r);
    let mut out: Vec<u8> = Vec::with_capacity(1 << 19);
    let (mut crc, mut total) = (0u32, 0u64);
    let mut flush = |out: &mut Vec<u8>, w: &mut W, all: bool| -> io::Result<()> {
        let keep = if all { 0 } else { WIN };
        if out.len() > keep {
            let n = out.len() - keep;
            crc = crc32(crc, &out[..n]);
            total += n as u64;
            w.write_all(&out[..n])?;
            out.drain(..n);
            if !progress(n as u64) {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
            }
        }
        Ok(())
    };
    let fixed = {
        let mut l = [8u8; 288];
        l[144..256].fill(9);
        l[256..280].fill(7);
        (Table::new(&l)?, Table::new(&[5u8; 30])?)
    };
    loop {
        let last = b.bits(1)? == 1;
        match b.bits(2)? {
            0 => {
                b.take(b.n % 8)?;
                let len = b.bits(16)? as usize;
                if b.bits(16)? as usize != !len & 0xFFFF {
                    return Err(bad("stored block length"));
                }
                b.bytes(len, &mut out)?;
            }
            t @ (1 | 2) => {
                let dynamic;
                let (lt, dt) = if t == 1 {
                    (&fixed.0, &fixed.1)
                } else {
                    let (hlit, hdist, hclen) = (b.bits(5)? as usize + 257, b.bits(5)? as usize + 1, b.bits(4)? as usize + 4);
                    if hlit > 286 || hdist > 30 {
                        return Err(bad("too many codes"));
                    }
                    let mut cl = [0u8; 19];
                    for &o in &CL_ORDER[..hclen] {
                        cl[o] = b.bits(3)? as u8;
                    }
                    let ct = Table::new(&cl)?;
                    let mut lens = vec![0u8; hlit + hdist];
                    let mut i = 0;
                    while i < lens.len() {
                        let s = ct.sym(&mut b)?;
                        let (v, rep) = match s {
                            0..=15 => (s as u8, 1),
                            16 => (*lens[..i].last().ok_or_else(|| bad("repeat with nothing before"))?, 3 + b.bits(2)? as usize),
                            17 => (0, 3 + b.bits(3)? as usize),
                            _ => (0, 11 + b.bits(7)? as usize),
                        };
                        if i + rep > lens.len() {
                            return Err(bad("code lengths overrun"));
                        }
                        lens[i..i + rep].fill(v);
                        i += rep;
                    }
                    dynamic = (Table::new(&lens[..hlit])?, Table::new(&lens[hlit..])?);
                    (&dynamic.0, &dynamic.1)
                };
                loop {
                    let s = lt.sym(&mut b)?;
                    if s < 256 {
                        out.push(s as u8);
                    } else if s == 256 {
                        break;
                    } else {
                        let c = s - 257;
                        if c >= 29 {
                            return Err(bad("bad length code"));
                        }
                        let len = LBASE[c] as usize + b.bits(LEXT[c] as u32)? as usize;
                        let d = dt.sym(&mut b)?;
                        if d >= 30 {
                            return Err(bad("bad distance code"));
                        }
                        let dist = DBASE[d] as usize + b.bits(DEXT[d] as u32)? as usize;
                        if dist > out.len() {
                            return Err(bad("distance reaches before the start"));
                        }
                        let from = out.len() - dist;
                        if dist >= len {
                            out.extend_from_within(from..from + len);
                        } else if dist == 1 {
                            let b = out[from];
                            out.resize(out.len() + len, b);
                        } else {
                            out.reserve(len);
                            for k in 0..len {
                                out.push(out[from + k]);
                            }
                        }
                    }
                    if out.len() >= FLUSH_AT {
                        flush(&mut out, w, false)?;
                    }
                }
            }
            _ => return Err(bad("bad block type")),
        }
        if last {
            break;
        }
    }
    flush(&mut out, w, true)?;
    Ok((crc, total))
}

const U32_MAX: u64 = 0xFFFF_FFFF;

pub struct Prepared {
    name: String,
    time: (u16, u16),
    method: u16,
    crc: u32,
    usize: u64,
    data: Vec<u8>,
}

impl Prepared {
    pub fn uncompressed_size(&self) -> u64 {
        self.usize
    }
}

pub fn prepare(name: String, path: &Path) -> io::Result<Prepared> {
    let raw = fs::read(path)?;
    let time = dos_time(path);
    let crc = crc32(0, &raw);
    let data = deflate(&raw);
    if raw.is_empty() || data.len() >= raw.len() {
        return Ok(Prepared { name, time, method: 0, crc, usize: raw.len() as u64, data: raw });
    }
    Ok(Prepared { name, time, method: 8, crc, usize: raw.len() as u64, data })
}

struct Cd {
    name: String,
    time: (u16, u16),
    method: u16,
    crc: u32,
    csize: u64,
    usize: u64,
    offset: u64,
    dir: bool,
}

pub struct Writer<W: Write> {
    w: W,
    pos: u64,
    cd: Vec<Cd>,
    limit: u64,
    hdr: Vec<u8>,
}

fn le16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn le32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn le64(v: &mut Vec<u8>, x: u64) {
    v.extend_from_slice(&x.to_le_bytes());
}

impl<W: Write> Writer<W> {
    pub fn new(w: W) -> Self {
        Writer { w, pos: 0, cd: vec![], limit: U32_MAX, hdr: Vec::with_capacity(128) }
    }

    #[cfg(test)]
    fn with_limit(mut self, limit: u64) -> Self {
        self.limit = limit;
        self
    }

    fn put(&mut self, b: &[u8]) -> io::Result<()> {
        self.w.write_all(b)?;
        self.pos += b.len() as u64;
        Ok(())
    }

    pub fn add_dir(&mut self, name: &str, time: (u16, u16)) -> io::Result<()> {
        let name = format!("{}/", name.trim_end_matches('/'));
        let offset = self.pos;
        let mut h = std::mem::take(&mut self.hdr);
        h.clear();
        local_header(&mut h, &name, time, 0, 0, 0, 0, false);
        let r = self.put(&h);
        self.hdr = h;
        r?;
        self.cd.push(Cd { name, time, method: 0, crc: 0, csize: 0, usize: 0, offset, dir: true });
        Ok(())
    }

    pub fn add_prepared(&mut self, f: Prepared) -> io::Result<()> {
        let offset = self.pos;
        let mut h = std::mem::take(&mut self.hdr);
        h.clear();
        local_header(&mut h, &f.name, f.time, f.method, f.crc, f.data.len() as u32, f.usize as u32, false);
        let r = self.put(&h);
        self.hdr = h;
        r?;
        self.put(&f.data)?;
        self.cd.push(Cd { name: f.name, time: f.time, method: f.method, crc: f.crc, csize: f.data.len() as u64, usize: f.usize, offset, dir: false });
        Ok(())
    }

    pub fn add_stream(&mut self, name: String, path: &Path, mut done: impl FnMut(u64) -> bool) -> io::Result<()> {
        let time = dos_time(path);
        let mut f = fs::File::open(path)?;
        let offset = self.pos;
        let mut h = std::mem::take(&mut self.hdr);
        h.clear();
        local_header(&mut h, &name, time, 8, 0, U32_MAX as u32, U32_MAX as u32, true);
        let r = self.put(&h);
        self.hdr = h;
        r?;
        let (mut d, mut out) = (Deflater::new(), Vec::new());
        let (mut crc, mut total, mut csize) = (0u32, 0u64, 0u64);
        let (tx_full, rx_full) = std::sync::mpsc::sync_channel::<io::Result<(Vec<u8>, usize)>>(2);
        let (tx_empty, rx_empty) = std::sync::mpsc::sync_channel::<Vec<u8>>(2);
        let _ = tx_empty.send(vec![0u8; CHUNK]);
        let _ = tx_empty.send(vec![0u8; CHUNK]);
        std::thread::scope(|s| -> io::Result<()> {
            s.spawn(move || {
                while let Ok(mut buf) = rx_empty.recv() {
                    let mut n = 0;
                    let r = loop {
                        if n == CHUNK {
                            break Ok(n);
                        }
                        match f.read(&mut buf[n..]) {
                            Ok(0) => break Ok(n),
                            Ok(k) => n += k,
                            Err(e) => break Err(e),
                        }
                    };
                    let last = !matches!(r, Ok(n) if n == CHUNK);
                    let msg = r.map(|n| (buf, n));
                    if tx_full.send(msg).is_err() || last {
                        break;
                    }
                }
            });
            for r in rx_full {
                let (buf, n) = r?;
                if n == 0 {
                    break;
                }
                crc = crc32(crc, &buf[..n]);
                total += n as u64;
                out.clear();
                d.chunk(&buf[..n], &mut out);
                let _ = tx_empty.send(buf);
                csize += out.len() as u64;
                self.put(&out)?;
                if !done(n as u64) {
                    return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
                }
            }
            Ok(())
        })?;
        out.clear();
        d.finish(&mut out);
        csize += out.len() as u64;
        self.put(&out)?;
        let mut dd = [0u8; 24];
        dd[0..4].copy_from_slice(&0x0807_4b50u32.to_le_bytes());
        dd[4..8].copy_from_slice(&crc.to_le_bytes());
        dd[8..16].copy_from_slice(&csize.to_le_bytes());
        dd[16..24].copy_from_slice(&total.to_le_bytes());
        self.put(&dd)?;
        self.cd.push(Cd { name, time, method: 8, crc, csize, usize: total, offset, dir: false });
        Ok(())
    }

    pub fn finish(mut self) -> io::Result<W> {
        let start = self.pos;
        let mut v = Vec::with_capacity(self.cd.len() * 64 + 128);
        for e in &self.cd {
            let mut x = [0u8; 24];
            let mut xlen = 0usize;
            let (us, cs, of) = (e.usize >= self.limit, e.csize >= self.limit, e.offset >= self.limit);
            if us {
                x[xlen..xlen + 8].copy_from_slice(&e.usize.to_le_bytes());
                xlen += 8;
            }
            if cs {
                x[xlen..xlen + 8].copy_from_slice(&e.csize.to_le_bytes());
                xlen += 8;
            }
            if of {
                x[xlen..xlen + 8].copy_from_slice(&e.offset.to_le_bytes());
                xlen += 8;
            }
            let big = |b: bool, v: u64| if b { U32_MAX as u32 } else { v as u32 };
            le32(&mut v, 0x0201_4b50);
            le16(&mut v, 45 | 10 << 8);
            le16(&mut v, if xlen == 0 { 20 } else { 45 });
            le16(&mut v, 0x0800);
            le16(&mut v, e.method);
            le16(&mut v, e.time.0);
            le16(&mut v, e.time.1);
            le32(&mut v, e.crc);
            le32(&mut v, big(cs, e.csize));
            le32(&mut v, big(us, e.usize));
            le16(&mut v, e.name.len() as u16);
            le16(&mut v, if xlen == 0 { 0 } else { xlen as u16 + 4 });
            le16(&mut v, 0);
            le16(&mut v, 0);
            le16(&mut v, 0);
            le32(&mut v, if e.dir { 0x10 } else { 0x20 });
            le32(&mut v, big(of, e.offset));
            v.extend_from_slice(e.name.as_bytes());
            if xlen > 0 {
                le16(&mut v, 1);
                le16(&mut v, xlen as u16);
                v.extend_from_slice(&x[..xlen]);
            }
        }
        let (n, size) = (self.cd.len() as u64, v.len() as u64);
        let z64 = n >= 0xFFFF || start >= self.limit || size >= self.limit;
        if z64 {
            let z = start + size;
            le32(&mut v, 0x0606_4b50);
            le64(&mut v, 44);
            le16(&mut v, 45);
            le16(&mut v, 45);
            le32(&mut v, 0);
            le32(&mut v, 0);
            le64(&mut v, n);
            le64(&mut v, n);
            le64(&mut v, size);
            le64(&mut v, start);
            le32(&mut v, 0x0706_4b50);
            le32(&mut v, 0);
            le64(&mut v, z);
            le32(&mut v, 1);
        }
        le32(&mut v, 0x0605_4b50);
        le16(&mut v, 0);
        le16(&mut v, 0);
        let sat = |v: u64, m: u64| if z64 { m } else { v };
        le16(&mut v, sat(n, 0xFFFF) as u16);
        le16(&mut v, sat(n, 0xFFFF) as u16);
        le32(&mut v, sat(size, U32_MAX) as u32);
        le32(&mut v, sat(start, U32_MAX) as u32);
        le16(&mut v, 0);
        self.put(&v)?;
        self.w.flush()?;
        Ok(self.w)
    }
}

#[allow(clippy::too_many_arguments)]
fn local_header(h: &mut Vec<u8>, name: &str, time: (u16, u16), method: u16, crc: u32, csize: u32, usize: u32, stream: bool) {
    le32(h, 0x0403_4b50);
    le16(h, if stream { 45 } else { 20 });
    le16(h, 0x0800 | if stream { 8 } else { 0 });
    le16(h, method);
    le16(h, time.0);
    le16(h, time.1);
    le32(h, crc);
    le32(h, csize);
    le32(h, usize);
    le16(h, name.len() as u16);
    le16(h, if stream { 20 } else { 0 });
    h.extend_from_slice(name.as_bytes());
    if stream {
        le16(h, 1);
        le16(h, 16);
        le64(h, 0);
        le64(h, 0);
    }
}

pub fn dos_time(p: &Path) -> (u16, u16) {
    use windows_sys::Win32::{Foundation::{FILETIME, SYSTEMTIME}, Storage::FileSystem::FileTimeToLocalFileTime, System::Time::FileTimeToSystemTime};
    let t = fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok());
    let Some(t) = t else { return (0, 0x21) };
    let ft100 = t.as_secs() * 10_000_000 + (t.subsec_nanos() / 100) as u64 + 116_444_736_000_000_000;
    let (ft, mut local, mut st) = (FILETIME { dwLowDateTime: ft100 as u32, dwHighDateTime: (ft100 >> 32) as u32 }, unsafe { std::mem::zeroed::<FILETIME>() }, unsafe { std::mem::zeroed::<SYSTEMTIME>() });
    unsafe {
        if FileTimeToLocalFileTime(&ft, &mut local) == 0 || FileTimeToSystemTime(&local, &mut st) == 0 || st.wYear < 1980 {
            return (0, 0x21);
        }
    }
    ((st.wHour << 11) | (st.wMinute << 5) | (st.wSecond / 2), ((st.wYear - 1980) << 9) | (st.wMonth << 5) | st.wDay)
}

fn from_dos(t: (u16, u16)) -> Option<SystemTime> {
    use windows_sys::Win32::{Foundation::{FILETIME, SYSTEMTIME}, Storage::FileSystem::LocalFileTimeToFileTime, System::Time::SystemTimeToFileTime};
    let (tm, dt) = t;
    if dt == 0 || (dt == 0x21 && tm == 0) {
        return None;
    }
    let st = SYSTEMTIME { wYear: (dt >> 9) + 1980, wMonth: (dt >> 5) & 15, wDayOfWeek: 0, wDay: dt & 31, wHour: tm >> 11, wMinute: (tm >> 5) & 63, wSecond: (tm & 31) * 2, wMilliseconds: 0 };
    let (mut local, mut utc) = unsafe { (std::mem::zeroed::<FILETIME>(), std::mem::zeroed::<FILETIME>()) };
    unsafe {
        if SystemTimeToFileTime(&st, &mut local) == 0 || LocalFileTimeToFileTime(&local, &mut utc) == 0 {
            return None;
        }
    }
    let t100 = (utc.dwHighDateTime as u64) << 32 | utc.dwLowDateTime as u64;
    t100.checked_sub(116_444_736_000_000_000).map(|x| UNIX_EPOCH + Duration::from_nanos(x * 100))
}

fn set_mtime(path: &Path, t: SystemTime) {
    use std::os::windows::fs::OpenOptionsExt;
    if let Ok(f) = fs::OpenOptions::new().write(true).custom_flags(0x0200_0000).open(path) {
        let _ = f.set_modified(t);
    }
}

struct Entry {
    name: String,
    time: (u16, u16),
    method: u16,
    flags: u16,
    crc: u32,
    csize: u64,
    usize: u64,
    offset: u64,
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}
fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], i: usize) -> u64 {
    u64::from_le_bytes(b[i..i + 8].try_into().unwrap())
}

fn read_at<F: Read + Seek>(f: &mut F, pos: u64, n: usize) -> io::Result<Vec<u8>> {
    f.seek(SeekFrom::Start(pos))?;
    let mut v = vec![0; n];
    f.read_exact(&mut v)?;
    Ok(v)
}

fn central_directory<F: Read + Seek>(f: &mut F) -> io::Result<Vec<Entry>> {
    let end = f.seek(SeekFrom::End(0))?;
    let tail_len = end.min(22 + 65535);
    let tail = read_at(f, end - tail_len, tail_len as usize)?;
    let at = (0..tail.len().saturating_sub(21)).rev().find(|&i| u32_at(&tail, i) == 0x0605_4b50).ok_or_else(|| bad("not a ZIP archive"))?;
    let eocd = end - tail_len + at as u64;
    let (mut count, mut size, mut start) = (u16_at(&tail, at + 10) as u64, u32_at(&tail, at + 12) as u64, u32_at(&tail, at + 16) as u64);
    if (count == 0xFFFF || size == U32_MAX || start == U32_MAX) && eocd >= 20 {
        let loc = read_at(f, eocd - 20, 20)?;
        if u32_at(&loc, 0) == 0x0706_4b50 {
            let z = read_at(f, u64_at(&loc, 8), 56)?;
            if u32_at(&z, 0) != 0x0606_4b50 {
                return Err(bad("broken ZIP64 directory"));
            }
            (count, size, start) = (u64_at(&z, 32), u64_at(&z, 40), u64_at(&z, 48));
        }
    }
    if size > 1 << 30 {
        return Err(bad("central directory is too big"));
    }
    let cd = read_at(f, start, size as usize)?;
    let (mut v, mut i) = (Vec::with_capacity((count as usize).min(cd.len() / 46)), 0usize);
    for _ in 0..count {
        if i + 46 > cd.len() || u32_at(&cd, i) != 0x0201_4b50 {
            return Err(bad("broken central directory"));
        }
        let (flags, method) = (u16_at(&cd, i + 8), u16_at(&cd, i + 10));
        let (nl, el, cl) = (u16_at(&cd, i + 28) as usize, u16_at(&cd, i + 30) as usize, u16_at(&cd, i + 32) as usize);
        if i + 46 + nl + el + cl > cd.len() {
            return Err(bad("broken central directory"));
        }
        let mut e = Entry {
            name: decode_name(&cd[i + 46..i + 46 + nl], flags),
            time: (u16_at(&cd, i + 12), u16_at(&cd, i + 14)),
            method,
            flags,
            crc: u32_at(&cd, i + 16),
            csize: u32_at(&cd, i + 20) as u64,
            usize: u32_at(&cd, i + 24) as u64,
            offset: u32_at(&cd, i + 42) as u64,
        };
        let mut x = &cd[i + 46 + nl..i + 46 + nl + el];
        while x.len() >= 4 {
            let (id, sz) = (u16_at(x, 0), u16_at(x, 2) as usize);
            if x.len() < 4 + sz {
                break;
            }
            if id == 1 {
                let mut f = &x[4..4 + sz];
                for field in [&mut e.usize, &mut e.csize, &mut e.offset] {
                    if *field == U32_MAX && f.len() >= 8 {
                        *field = u64_at(f, 0);
                        f = &f[8..];
                    }
                }
            }
            x = &x[4 + sz..];
        }
        v.push(e);
        i += 46 + nl + el + cl;
    }
    Ok(v)
}

const CP437_HIGH_CHARS: [char; 128] = [
    'Ç', 'ü', 'é', 'â', 'ä', 'à', 'å', 'ç', 'ê', 'ë', 'è', 'ï', 'î', 'ì', 'Ä', 'Å',
    'É', 'æ', 'Æ', 'ô', 'ö', 'ò', 'û', 'ù', 'ÿ', 'Ö', 'Ü', '¢', '£', '¥', '₧', 'ƒ',
    'á', 'í', 'ó', 'ú', 'ñ', 'Ñ', 'ª', 'º', '¿', '⌐', '¬', '½', '¼', '¡', '«', '»',
    '░', '▒', '▓', '│', '┤', '╡', '╢', '╖', '╕', '╣', '║', '╗', '╝', '╜', '╛', '┐',
    '└', '┴', '┬', '├', '─', '┼', '╞', '╟', '╚', '╔', '╩', '╦', '╠', '═', '╬', '╧',
    '╨', '╤', '╥', '╙', '╘', '╒', '╓', '╫', '╪', '┘', '┌', '█', '▄', '▌', '▐', '▀',
    'α', 'ß', 'Γ', 'π', 'Σ', 'σ', 'µ', 'τ', 'Φ', 'Θ', 'Ω', 'δ', '∞', 'φ', 'ε', '∩',
    '≡', '±', '≥', '≤', '⌠', '⌡', '÷', '≈', '°', '∙', '·', '√', 'ⁿ', '²', '■', '\u{a0}',
];

fn decode_name(b: &[u8], flags: u16) -> String {
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) if flags & 0x800 != 0 => String::from_utf8_lossy(b).into_owned(),
        Err(_) => b.iter().map(|&c| if c < 0x80 { c as char } else { CP437_HIGH_CHARS[c as usize - 0x80] }).collect(),
    }
}

fn safe_path(out: &Path, name: &str) -> Result<Option<PathBuf>, String> {
    let mut p = out.to_path_buf();
    let mut any = false;
    for c in name.replace('\\', "/").split('/') {
        match c {
            "" | "." => continue,
            ".." => return Err(format!("{name}: the name leads out of the folder")),
            _ if Path::new(c).components().any(|k| !matches!(k, Component::Normal(_))) || c.contains(':') => return Err(format!("{name}: bad name")),
            _ => {
                p.push(c);
                any = true;
            }
        }
    }
    Ok(any.then_some(p))
}

pub struct Item {
    pub name: String,
    pub dir: bool,
    pub size: u64,
    pub time: (u16, u16),
}

pub fn list_dir(archive: &Path, inner: &str) -> Result<Vec<Item>, String> {
    let mut f = fs::File::open(archive).map_err(|e| e.to_string())?;
    let entries = central_directory(&mut f).map_err(|e| e.to_string())?;
    let prefix = if inner.is_empty() { String::new() } else { format!("{}/", inner.trim_matches('/')) };
    let mut m: std::collections::BTreeMap<String, Item> = Default::default();
    for e in entries {
        let name = e.name.replace('\\', "/");
        let Some(rest) = name.trim_start_matches('/').strip_prefix(&prefix) else { continue };
        let Some(first) = rest.split('/').next().filter(|f| !f.is_empty()) else { continue };
        let deeper = rest.trim_end_matches('/').len() > first.len() || rest.ends_with('/');
        let item = m.entry(first.to_string()).or_insert(Item { name: first.to_string(), dir: deeper, size: 0, time: (0, 0x21) });
        if deeper {
            item.dir = true;
            if rest.trim_end_matches('/') == first { item.time = e.time; }
        } else {
            item.size = e.usize;
            item.time = e.time;
        }
    }
    Ok(m.into_values().collect())
}

pub fn split_zip(p: &Path) -> Option<(PathBuf, String)> {
    let raw = p.as_os_str().as_encoded_bytes();
    if !raw.windows(4).any(|w| w.eq_ignore_ascii_case(b".zip")) {
        return None;
    }
    let mut found = None;
    for a in p.ancestors() {
        if a.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")) && a.is_file() {
            found = Some(a);
        }
    }
    let a = found?;
    let inner = p.strip_prefix(a).ok()?.to_string_lossy().replace('\\', "/");
    Some((a.to_path_buf(), inner))
}

pub fn dos_to_filetime(t: (u16, u16)) -> u64 {
    from_dos(t).and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() * 10_000_000 + 116_444_736_000_000_000)
}

pub fn extract(archive: &Path, out: &Path, total: impl FnOnce(u64), step: impl FnMut(u64) -> bool) -> Result<(), String> {
    extract_where(archive, out, |n| Some(n.to_string()), total, step)
}

pub fn extract_some(archive: &Path, names: &[String], out: &Path) -> Result<(), String> {
    let pick = |n: &str| {
        let n = n.replace('\\', "/");
        let n = n.trim_start_matches('/');
        names.iter().find_map(|sel| {
            let sel = sel.trim_matches('/');
            let cut = sel.rfind('/').map_or(0, |i| i + 1);
            (n.trim_end_matches('/') == sel || n.starts_with(&format!("{sel}/"))).then(|| n[cut..].to_string())
        })
    };
    extract_where(archive, out, pick, |_| {}, |_| true)
}

fn extract_where(archive: &Path, out: &Path, pick: impl Fn(&str) -> Option<String>, total: impl FnOnce(u64), mut step: impl FnMut(u64) -> bool) -> Result<(), String> {
    let mut f = fs::File::open(archive).map_err(|e| e.to_string())?;
    let entries = central_directory(&mut f).map_err(|e| e.to_string())?;
    fs::create_dir_all(out).map_err(|e| e.to_string())?;
    total(entries.iter().map(|e| e.usize).sum());
    let mut dirs = vec![];
    for e in entries {
        let Some(rel) = pick(&e.name) else { continue };
        let Some(path) = safe_path(out, &rel)? else { continue };
        if e.name.ends_with('/') || e.name.ends_with('\\') {
            fs::create_dir_all(&path).map_err(|x| format!("{}: {x}", e.name))?;
            dirs.push((path, from_dos(e.time)));
            continue;
        }
        if e.flags & 1 != 0 {
            return Err(format!("{}: encrypted entries are not supported", e.name));
        }
        let ctx = |x: io::Error| if x.kind() == io::ErrorKind::Interrupted { "cancelled".to_string() } else { format!("{}: {x}", e.name) };
        let lh = read_at(&mut f, e.offset, 30).map_err(ctx)?;
        if u32_at(&lh, 0) != 0x0403_4b50 {
            return Err(format!("{}: broken local header", e.name));
        }
        let data = e.offset + 30 + u16_at(&lh, 26) as u64 + u16_at(&lh, 28) as u64;
        f.seek(SeekFrom::Start(data)).map_err(ctx)?;
        if let Some(d) = path.parent() {
            fs::create_dir_all(d).map_err(ctx)?;
        }
        let mut w = BufWriter::with_capacity(1 << 20, fs::File::create(&path).map_err(ctx)?);
        let src = io::BufReader::with_capacity(1 << 16, (&mut f).take(e.csize));
        let (crc, size) = match e.method {
            0 => {
                let mut src = src;
                let (mut crc, mut n, mut buf) = (0u32, 0u64, vec![0u8; 1 << 16]);
                loop {
                    let k = src.read(&mut buf).map_err(ctx)?;
                    if k == 0 {
                        break;
                    }
                    crc = crc32(crc, &buf[..k]);
                    n += k as u64;
                    w.write_all(&buf[..k]).map_err(ctx)?;
                    if !step(k as u64) {
                        return Err("cancelled".into());
                    }
                }
                (crc, n)
            }
            8 => inflate(src, &mut w, &mut step).map_err(ctx)?,
            m => return Err(format!("{}: compression method {m} is not supported", e.name)),
        };
        let file = w.into_inner().map_err(|x| ctx(x.into_error()))?;
        if let Some(t) = from_dos(e.time) {
            let _ = file.set_modified(t);
        }
        drop(file);
        if crc != e.crc || size != e.usize {
            return Err(format!("{}: the data is damaged (checksum or size differs)", e.name));
        }
    }
    for (p, t) in dirs.into_iter().rev().filter_map(|(p, t)| Some((p, t?))) {
        set_mtime(&p, t);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn sample(n: usize, seed: u32) -> Vec<u8> {
        let words = ["alpha", "beta", "gamma", "delta", "epsilon", " ", "\n", "0123456789", "zip"];
        let (mut s, mut v) = (seed, Vec::new());
        while v.len() < n {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            v.extend_from_slice(words[(s >> 24) as usize % words.len()].as_bytes());
            if s >> 20 & 7 == 0 {
                v.push((s >> 8) as u8);
            }
        }
        v.truncate(n);
        v
    }

    fn noise(n: usize) -> Vec<u8> {
        let mut s = 12345u32;
        (0..n).map(|_| { s = s.wrapping_mul(1103515245).wrapping_add(12345); (s >> 16) as u8 }).collect()
    }

    fn round(data: &[u8]) {
        let z = deflate(data);
        let mut back = vec![];
        let (crc, n) = inflate(&z[..], &mut back, &mut |_| true).unwrap();
        assert_eq!(back, data);
        assert_eq!((crc, n), (crc32(0, data), data.len() as u64));
    }

    #[test]
    fn crc_known_value() {
        assert_eq!(crc32(0, b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(crc32(0, b"1234"), b"56789"), 0xCBF4_3926);
    }

    #[test]
    fn deflate_round_trips() {
        round(b"");
        round(b"a");
        round(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        round(&sample(10_000, 1));
        round(&sample(3_000_000, 2));
        round(&noise(200_000));
        round(&vec![7u8; 300_000]);
        let mut mixed = sample(100_000, 3);
        mixed.extend(noise(70_000));
        mixed.extend(sample(100_000, 4));
        round(&mixed);
    }

    #[test]
    fn compresses_text_and_does_not_grow_noise() {
        let t = sample(1_000_000, 5);
        assert!(deflate(&t).len() < t.len() / 3, "text should shrink a lot");
        let n = noise(1_000_000);
        assert!(deflate(&n).len() < n.len() + 1000, "noise stays near its size");
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("zipx-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn archive_round_trip_and_sizes() {
        let d = tmp("rt");
        fs::write(d.join("text.txt"), sample(500_000, 6)).unwrap();
        fs::write(d.join("noise.bin"), noise(100_000)).unwrap();
        fs::write(d.join("empty"), b"").unwrap();
        fs::write(d.join("big.txt"), sample(2_500_000, 7)).unwrap();
        let mut w = Writer::new(fs::File::create(d.join("o.zip")).unwrap());
        w.add_dir("dir", (0, 0x21)).unwrap();
        for n in ["text.txt", "noise.bin", "empty"] {
            w.add_prepared(prepare(format!("dir/{n}"), &d.join(n)).unwrap()).unwrap();
        }
        let mut seen = 0;
        w.add_stream("dir/big.txt".into(), &d.join("big.txt"), |n| { seen += n; true }).unwrap();
        assert_eq!(seen, 2_500_000);
        w.finish().unwrap();
        extract(&d.join("o.zip"), &d.join("back"), |_| {}, |_| true).unwrap();
        for n in ["text.txt", "noise.bin", "empty", "big.txt"] {
            assert_eq!(fs::read(d.join(n)).unwrap(), fs::read(d.join("back/dir").join(n)).unwrap(), "{n}");
        }
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn rejects_zip_slip_and_damage() {
        let o = Path::new(r"C:\x\out");
        assert!(safe_path(o, "../evil.txt").is_err());
        assert!(safe_path(o, r"a\..\..\evil.txt").is_err());
        assert!(safe_path(o, "C:/evil.txt").is_err());
        assert!(safe_path(o, "a:b.txt").is_err());
        assert_eq!(safe_path(o, "/abs/ok.txt").unwrap(), Some(o.join("abs").join("ok.txt")));
        assert_eq!(safe_path(o, "").unwrap(), None);
        let d = tmp("bad");
        fs::write(d.join("a.txt"), sample(50_000, 8)).unwrap();
        let mut w = Writer::new(fs::File::create(d.join("o.zip")).unwrap());
        w.add_prepared(prepare("a.txt".into(), &d.join("a.txt")).unwrap()).unwrap();
        w.finish().unwrap();
        let mut b = fs::read(d.join("o.zip")).unwrap();
        b[200] ^= 0x55;
        fs::write(d.join("bad.zip"), b).unwrap();
        assert!(extract(&d.join("bad.zip"), &d.join("out"), |_| {}, |_| true).is_err());
        assert!(extract(&d.join("a.txt"), &d.join("out2"), |_| {}, |_| true).is_err(), "not an archive");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn matches_reach_across_chunks() {
        let tail = sample(20_000, 11);
        let mut d = sample(CHUNK - tail.len(), 12);
        d.extend(&tail);
        d.extend(&tail);
        d.extend(sample(5000, 13));
        round(&d);
        let (a, b) = (deflate(&d).len(), deflate(&d[..CHUNK]).len() + deflate(&d[CHUNK..]).len());
        assert!(a + 3_000 < b, "{a} vs {b}: the repeat should cost almost nothing");
    }

    #[test]
    fn random_looking_data_is_stored_fast() {
        let n = noise(3_000_000);
        let t = std::time::Instant::now();
        let z = deflate(&n);
        assert!(z.len() < n.len() + 400, "stored blocks add only headers");
        assert!(t.elapsed() < std::time::Duration::from_millis(500));
        round(&n);
    }

    #[test]
    fn zip64_fields_with_a_small_limit() {
        let d = tmp("z64");
        fs::write(d.join("a.txt"), sample(5000, 21)).unwrap();
        fs::write(d.join("b.txt"), sample(9000, 22)).unwrap();
        let mut w = Writer::new(fs::File::create(d.join("o.zip")).unwrap()).with_limit(100);
        w.add_dir("dir", (0, 0x21)).unwrap();
        w.add_prepared(prepare("dir/a.txt".into(), &d.join("a.txt")).unwrap()).unwrap();
        w.add_stream("dir/b.txt".into(), &d.join("b.txt"), |_| true).unwrap();
        w.finish().unwrap();
        extract(&d.join("o.zip"), &d.join("back"), |_| {}, |_| true).unwrap();
        assert_eq!(fs::read(d.join("a.txt")).unwrap(), fs::read(d.join("back/dir/a.txt")).unwrap());
        assert_eq!(fs::read(d.join("b.txt")).unwrap(), fs::read(d.join("back/dir/b.txt")).unwrap());
        let mut z = zip::ZipArchive::new(fs::File::open(d.join("o.zip")).unwrap()).unwrap();
        for (n, f) in [("dir/a.txt", "a.txt"), ("dir/b.txt", "b.txt")] {
            let mut got = vec![];
            z.by_name(n).unwrap().read_to_end(&mut got).unwrap();
            assert_eq!(got, fs::read(d.join(f)).unwrap(), "{n}");
        }
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn times_survive_a_round_trip() {
        let d = tmp("time");
        fs::create_dir(d.join("sub")).unwrap();
        fs::write(d.join("sub/f.txt"), b"x").unwrap();
        let then = UNIX_EPOCH + Duration::from_secs(1_500_000_000);
        let f = fs::OpenOptions::new().write(true).open(d.join("sub/f.txt")).unwrap();
        f.set_modified(then).unwrap();
        drop(f);
        set_mtime(&d.join("sub"), then);
        let mut w = Writer::new(fs::File::create(d.join("o.zip")).unwrap());
        w.add_dir("sub", dos_time(&d.join("sub"))).unwrap();
        w.add_prepared(prepare("sub/f.txt".into(), &d.join("sub/f.txt")).unwrap()).unwrap();
        w.finish().unwrap();
        extract(&d.join("o.zip"), &d.join("back"), |_| {}, |_| true).unwrap();
        for p in ["back/sub/f.txt", "back/sub"] {
            let got = fs::metadata(d.join(p)).unwrap().modified().unwrap();
            assert_eq!(got, then, "{p}");
        }
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn old_dos_names_and_cancel() {
        assert_eq!(CP437_HIGH_CHARS.len(), 128);
        assert_eq!(decode_name(&[0x82, b'a', 0x94], 0), "éaö");
        assert_eq!(decode_name("é.txt".as_bytes(), 0), "é.txt");
        assert_eq!(decode_name(&[b'a', 0xFF], 0x800), "a\u{FFFD}");
        let d = tmp("cancel");
        fs::write(d.join("a.txt"), sample(3_000_000, 31)).unwrap();
        let mut w = Writer::new(fs::File::create(d.join("o.zip")).unwrap());
        w.add_prepared(prepare("a.txt".into(), &d.join("a.txt")).unwrap()).unwrap();
        w.finish().unwrap();
        let (mut total, mut seen) = (0, 0);
        let r = extract(&d.join("o.zip"), &d.join("out"), |t| total = t, |n| { seen += n; false });
        assert_eq!(r, Err("cancelled".to_string()));
        assert_eq!(total, 3_000_000);
        assert!(seen > 0 && seen < 3_000_000);
        let mut w = Writer::new(fs::File::create(d.join("p.zip")).unwrap());
        let e = w.add_stream("a.txt".into(), &d.join("a.txt"), |_| false).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::Interrupted);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn browses_an_archive_like_folders() {
        let d = tmp("browse");
        for (n, c) in [("a.txt", "A"), ("b.txt", "BB")] {
            fs::write(d.join(n), c).unwrap();
        }
        let mut w = Writer::new(fs::File::create(d.join("o.zip")).unwrap());
        w.add_prepared(prepare("top.txt".into(), &d.join("a.txt")).unwrap()).unwrap();
        w.add_prepared(prepare("dir/sub/deep.txt".into(), &d.join("b.txt")).unwrap()).unwrap();
        w.add_prepared(prepare("dir/mid.txt".into(), &d.join("a.txt")).unwrap()).unwrap();
        w.add_dir("empty", (0, 0x21)).unwrap();
        w.finish().unwrap();
        let z = d.join("o.zip");
        let names = |inner: &str| list_dir(&z, inner).unwrap().into_iter().map(|i| format!("{}{}", i.name, if i.dir { "/" } else { "" })).collect::<Vec<_>>();
        assert_eq!(names(""), ["dir/", "empty/", "top.txt"]);
        assert_eq!(names("dir"), ["mid.txt", "sub/"]);
        assert_eq!(names("dir/sub"), ["deep.txt"]);
        assert!(names("nope").is_empty());
        assert_eq!(list_dir(&z, "dir/sub").unwrap()[0].size, 1 + 1);
        extract_some(&z, &["dir/sub/deep.txt".to_string()], &d.join("o1")).unwrap();
        extract_some(&z, &["dir".to_string(), "top.txt".to_string()], &d.join("o2")).unwrap();
        assert_eq!(fs::read_to_string(d.join("o1/deep.txt")).unwrap(), "BB");
        assert_eq!(fs::read_to_string(d.join("o2/dir/sub/deep.txt")).unwrap(), "BB");
        assert!(d.join("o2/dir/mid.txt").is_file() && d.join("o2/top.txt").is_file() && !d.join("o2/empty").exists());
        assert_eq!(split_zip(&z.join("dir").join("sub")), Some((z.clone(), "dir/sub".to_string())));
        assert_eq!(split_zip(&z), Some((z.clone(), String::new())));
        assert_eq!(split_zip(&d), None);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    #[ignore]
    fn bench() {
        use std::io::Write;
        let mut text = vec![];
        let mut stack = vec![PathBuf::from(r"..\\..\\plugins"), PathBuf::from(r"..\\..\\src")];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() && p.file_name().is_some_and(|n| n != "target") {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    text.extend(fs::read(p).unwrap());
                }
            }
        }
        let mut exe = fs::read(r"C:\Windows\System32\kernel32.dll").unwrap_or_default();
        exe.truncate(2 << 20);
        for (name, data) in [("sources", text), ("dll", exe), ("noise", noise(2 << 20))] {
            if data.is_empty() {
                continue;
            }
            let t = std::time::Instant::now();
            let ours = deflate(&data);
            let t_ours = t.elapsed();
            let t = std::time::Instant::now();
            let mut w = zip::ZipWriter::new(io::Cursor::new(vec![]));
            w.start_file("a", zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)).unwrap();
            w.write_all(&data).unwrap();
            let theirs = w.finish().unwrap().into_inner().len();
            let t_theirs = t.elapsed();
            let t = std::time::Instant::now();
            let mut back = vec![];
            inflate(&ours[..], &mut back, &mut |_| true).unwrap();
            println!("BENCH {name} {} B: ours {} B in {:?} (inflate {:?}), zip crate {} B in {:?}", data.len(), ours.len(), t_ours, t.elapsed(), theirs, t_theirs);
        }
    }
}
