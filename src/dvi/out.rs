//! Writing a DVI file the way tex writes one: `tex.web` §583-§643.
//!
//! `crate::dvi::Writer` writes a correct DVI file; this writes TeX's. The
//! difference is §607-§615's `movement`, which looks back through the
//! movements already written for one of the same size and, when it finds one
//! it may, rewrites that earlier `down` or `right` into a `y1`/`z1` (or
//! `w1`/`x1`) and uses `y0`/`z0` here -- and §601's `dvi_pop`, which takes back
//! a `push` that nothing followed. Both depend on WHICH bytes are still in
//! tex's output buffer (`dvi_gone`, §595), so the buffer's halving is kept
//! exactly as §597-§598 keep it, with TeX Live's `dvi_buf_size` of 16384.
//! Fonts are defined where a page first uses them (§621) and again in the
//! postamble, highest internal number first (§643).

/// TeX Live's `dvi_buf_size` (texmf.cnf), which decides when an earlier
/// movement can no longer be rewritten.
const DVI_BUF_SIZE: usize = 16384;
const HALF_BUF: usize = DVI_BUF_SIZE / 2;

/// §586's opcodes.
const SET1: u8 = 128;
const SET_RULE: u8 = 132;
const PUT_RULE: u8 = 137;
const BOP: u8 = 139;
const EOP: u8 = 140;
const PUSH: u8 = 141;
const POP: u8 = 142;
const RIGHT1: u8 = 143;
#[cfg(test)]
const W0: u8 = 147;
const DOWN1: u8 = 157;
const Y0: u8 = 161;
const Y1: u8 = 162;
const Z0: u8 = 166;
const Z1: u8 = 167;
const FNT_NUM_0: u8 = 171;
const FNT1: u8 = 235;
const XXX1: u8 = 239;
const XXX4: u8 = 242;
const FNT_DEF1: u8 = 243;
const PRE: u8 = 247;
const POST: u8 = 248;
const POST_POST: u8 = 249;
const ID_BYTE: u8 = 2;

/// §608's `info` of a movement-stack entry.
const Y_HERE: u8 = 1;
const Z_HERE: u8 = 2;
const YZ_OK: u8 = 3;
const Y_OK: u8 = 4;
const Z_OK: u8 = 5;
const D_FIXED: u8 = 6;

/// §611's states.
const NONE_SEEN: u8 = 0;
const Y_SEEN: u8 = 6;
const Z_SEEN: u8 = 12;

/// One entry of the down or right stack (§605).
#[derive(Clone, Copy, Debug)]
struct Movement {
    width: i32,
    location: usize,
    info: u8,
}

/// A font as a `fnt_def` describes it (§602).
#[derive(Clone, Debug)]
pub struct FontDef {
    pub checksum: u32,
    pub size: i32,
    pub design_size: i32,
    pub area: String,
    pub name: String,
}

/// tex's DVI output state.
pub struct Out {
    bytes: Vec<u8>,
    /// §595: `dvi_ptr`, `dvi_limit`, `dvi_offset` and `dvi_gone`.
    dvi_ptr: usize,
    dvi_limit: usize,
    dvi_offset: usize,
    dvi_gone: usize,
    /// §605: top of stack is the LAST element.
    down: Vec<Movement>,
    right: Vec<Movement>,
    /// Internal font numbers as tex numbers them (`\nullfont` is 0), and
    /// §549's `font_used`.
    fonts: Vec<Option<FontDef>>,
    font_used: Vec<bool>,
    /// §592.
    last_bop: i32,
    pub total_pages: i32,
    pub max_push: i32,
    pub max_v: i32,
    pub max_h: i32,
    mag: i32,
}

impl Out {
    /// A file whose preamble (§617) carries `comment` -- tex's is ` TeX
    /// output YYYY.MM.DD:HHMM` -- and whose magnification is `mag`.
    pub fn new(comment: &str, mag: i32, fonts: Vec<Option<FontDef>>) -> Out {
        let used = vec![false; fonts.len()];
        let mut o = Out {
            bytes: Vec::new(),
            dvi_ptr: 0,
            dvi_limit: DVI_BUF_SIZE,
            dvi_offset: 0,
            dvi_gone: 0,
            down: Vec::new(),
            right: Vec::new(),
            fonts,
            font_used: used,
            last_bop: -1,
            total_pages: 0,
            max_push: 0,
            max_v: 0,
            max_h: 0,
            mag,
        };
        o.out(PRE);
        o.out(ID_BYTE);
        o.four(25_400_000);
        o.four(473_628_672);
        o.four(mag);
        let c: Vec<u8> = comment.bytes().take(255).collect();
        o.out(c.len() as u8);
        for b in c {
            o.out(b);
        }
        o
    }

    /// `dvi_out` (§598) with §598's `dvi_swap`.
    fn out(&mut self, b: u8) {
        self.bytes.push(b);
        self.dvi_ptr += 1;
        if self.dvi_ptr == self.dvi_limit {
            if self.dvi_limit == DVI_BUF_SIZE {
                self.dvi_limit = HALF_BUF;
                self.dvi_offset += DVI_BUF_SIZE;
                self.dvi_ptr = 0;
            } else {
                self.dvi_limit = DVI_BUF_SIZE;
            }
            self.dvi_gone += HALF_BUF;
        }
    }

    /// The byte number of the next byte (`dvi_offset+dvi_ptr`).
    pub fn here(&self) -> usize {
        self.bytes.len()
    }

    /// `dvi_four` (§600).
    fn four(&mut self, x: i32) {
        for b in x.to_be_bytes() {
            self.out(b);
        }
    }

    /// `dvi_font_def` (§602).
    fn font_def(&mut self, f: usize) {
        let Some(Some(def)) = self.fonts.get(f).cloned() else {
            return;
        };
        self.out(FNT_DEF1);
        self.out((f - 1) as u8);
        for b in def.checksum.to_be_bytes() {
            self.out(b);
        }
        self.four(def.size);
        self.four(def.design_size);
        self.out(def.area.len() as u8);
        self.out(def.name.len() as u8);
        for b in def.area.bytes().chain(def.name.bytes()) {
            self.out(b);
        }
    }

    /// §621: select font `f`, defining it the first time.
    pub fn font(&mut self, f: usize) {
        if f >= self.font_used.len() {
            self.font_used.resize(f + 1, false);
        }
        if !self.font_used[f] {
            self.font_def(f);
            self.font_used[f] = true;
        }
        if f <= 64 {
            self.out((f - 1) as u8 + FNT_NUM_0);
        } else {
            self.out(FNT1);
            self.out((f - 1) as u8);
        }
    }

    /// §620: one character, `set_char` or `set1`.
    pub fn set_char(&mut self, c: u32) {
        if c >= 128 {
            self.out(SET1);
        }
        self.out(c as u8);
    }

    /// §624 (`set_rule`) and §633 (`put_rule`).
    pub fn rule(&mut self, height: i32, width: i32, set: bool) {
        self.out(if set { SET_RULE } else { PUT_RULE });
        self.four(height);
        self.four(width);
    }

    /// §1368's `special_out`: `xxx1` or `xxx4`.
    pub fn special(&mut self, text: &[u8]) {
        if text.len() < 256 {
            self.out(XXX1);
            self.out(text.len() as u8);
        } else {
            self.out(XXX4);
            self.four(text.len() as i32);
        }
        for &b in text {
            self.out(b);
        }
    }

    /// `push`, returning §619's `save_loc`.
    pub fn push(&mut self) -> usize {
        self.out(PUSH);
        self.here()
    }

    /// `dvi_pop(l)` (§601): a push with nothing after it is taken back
    /// rather than closed.
    pub fn pop(&mut self, save_loc: usize) {
        if save_loc == self.here() && self.dvi_ptr > 0 {
            self.bytes.pop();
            self.dvi_ptr -= 1;
        } else {
            self.out(POP);
        }
    }

    /// `prune_movements(l)` (§615): the movements made since `l` are
    /// forgotten, so a later one cannot reuse them.
    pub fn prune(&mut self, l: usize) {
        while self.down.last().is_some_and(|m| m.location >= l) {
            self.down.pop();
        }
        while self.right.last().is_some_and(|m| m.location >= l) {
            self.right.pop();
        }
    }

    pub fn right(&mut self, w: i32) {
        self.movement(w, RIGHT1);
    }

    pub fn down(&mut self, w: i32) {
        self.movement(w, DOWN1);
    }

    /// `movement(w, o)` (§607-§614).
    fn movement(&mut self, w: i32, o: u8) {
        let location = self.here();
        let stack = match o {
            DOWN1 => &mut self.down,
            _ => &mut self.right,
        };
        stack.push(Movement {
            width: w,
            location,
            info: YZ_OK,
        });
        let q = stack.len() - 1;
        // §611: look down the stack.
        let mut mstate = NONE_SEEN;
        let mut found: Option<usize> = None;
        let mut i = q;
        'look: while i > 0 {
            i -= 1;
            let p = stack[i];
            if p.width == w {
                // §612.
                match mstate + p.info {
                    s if s == NONE_SEEN + YZ_OK
                        || s == NONE_SEEN + Y_OK
                        || s == Z_SEEN + YZ_OK
                        || s == Z_SEEN + Y_OK =>
                    {
                        if p.location < self.dvi_gone {
                            break 'look;
                        }
                        // §613: the earlier command becomes `y1` or `w1`.
                        self.bytes[p.location] += Y1 - DOWN1;
                        stack[i].info = Y_HERE;
                        found = Some(i);
                        break 'look;
                    }
                    s if s == NONE_SEEN + Z_OK || s == Y_SEEN + YZ_OK || s == Y_SEEN + Z_OK => {
                        if p.location < self.dvi_gone {
                            break 'look;
                        }
                        // §614: the earlier command becomes `z1` or `x1`.
                        self.bytes[p.location] += Z1 - DOWN1;
                        stack[i].info = Z_HERE;
                        found = Some(i);
                        break 'look;
                    }
                    s if s == NONE_SEEN + Y_HERE
                        || s == NONE_SEEN + Z_HERE
                        || s == Y_SEEN + Z_HERE
                        || s == Z_SEEN + Y_HERE =>
                    {
                        found = Some(i);
                        break 'look;
                    }
                    _ => {}
                }
            } else {
                match mstate + p.info {
                    s if s == NONE_SEEN + Y_HERE => mstate = Y_SEEN,
                    s if s == NONE_SEEN + Z_HERE => mstate = Z_SEEN,
                    s if s == Y_SEEN + Z_HERE || s == Z_SEEN + Y_HERE => break 'look,
                    _ => {}
                }
            }
        }
        if let Some(p) = found {
            // §609: reuse the earlier appearance of `w`.
            let info = stack[p].info;
            stack[q].info = info;
            if info == Y_HERE {
                for e in stack[p + 1..q].iter_mut() {
                    e.info = match e.info {
                        YZ_OK => Z_OK,
                        Y_OK => D_FIXED,
                        other => other,
                    };
                }
                self.out(o + (Y0 - DOWN1));
            } else {
                for e in stack[p + 1..q].iter_mut() {
                    e.info = match e.info {
                        YZ_OK => Y_OK,
                        Z_OK => D_FIXED,
                        other => other,
                    };
                }
                self.out(o + (Z0 - DOWN1));
            }
            return;
        }
        // §610: a `down` or `right` in as few bytes as it fits.
        let a = w.unsigned_abs();
        if a >= 0o40000000 {
            self.out(o + 3);
            self.four(w);
            return;
        }
        let mut w = i64::from(w);
        if a >= 0o100000 {
            self.out(o + 2);
            if w < 0 {
                w += 0o100000000;
            }
            self.out((w / 0o200000) as u8);
            w %= 0o200000;
            self.out((w / 0o400) as u8);
            self.out((w % 0o400) as u8);
            return;
        }
        if a >= 0o200 {
            self.out(o + 1);
            if w < 0 {
                w += 0o200000;
            }
            self.out((w / 0o400) as u8);
            self.out((w % 0o400) as u8);
            return;
        }
        self.out(o);
        if w < 0 {
            w += 0o400;
        }
        self.out((w % 0o400) as u8);
    }

    /// §640: `bop`, the ten counts, and the pointer to the previous page.
    pub fn begin_page(&mut self, counts: [i32; 10]) {
        let page_loc = self.here() as i32;
        self.out(BOP);
        for c in counts {
            self.four(c);
        }
        let last = self.last_bop;
        self.four(last);
        self.last_bop = page_loc;
    }

    /// §640: `eop`.
    pub fn end_page(&mut self) {
        self.out(EOP);
        self.total_pages += 1;
    }

    /// §642-§643: the postamble, and the file.
    pub fn finish(mut self) -> Vec<u8> {
        self.out(POST);
        let last = self.last_bop;
        self.four(last);
        self.last_bop = self.here() as i32 - 5;
        self.four(25_400_000);
        self.four(473_628_672);
        let mag = self.mag;
        self.four(mag);
        let (v, h) = (self.max_v, self.max_h);
        self.four(v);
        self.four(h);
        self.out((self.max_push / 256) as u8);
        self.out((self.max_push % 256) as u8);
        self.out(((self.total_pages / 256) % 256) as u8);
        self.out((self.total_pages % 256) as u8);
        for f in (1..self.font_used.len()).rev() {
            if self.font_used[f] {
                self.font_def(f);
            }
        }
        self.out(POST_POST);
        let last = self.last_bop;
        self.four(last);
        self.out(ID_BYTE);
        let k = 4 + ((DVI_BUF_SIZE - self.dvi_ptr % DVI_BUF_SIZE) % 4);
        for _ in 0..k {
            self.out(223);
        }
        self.bytes
    }
}

impl crate::shipout::Sink for Out {
    fn here(&self) -> usize {
        Out::here(self)
    }
    fn push(&mut self) -> usize {
        Out::push(self)
    }
    fn pop(&mut self, save_loc: usize) {
        Out::pop(self, save_loc)
    }
    fn prune(&mut self, save_loc: usize) {
        Out::prune(self, save_loc)
    }
    fn right(&mut self, w: i32) {
        Out::right(self, w)
    }
    fn down(&mut self, w: i32) {
        Out::down(self, w)
    }
    fn font(&mut self, f: usize) {
        Out::font(self, f)
    }
    fn set_char(&mut self, c: u32, _width: i32) {
        Out::set_char(self, c)
    }
    fn rule(&mut self, height: i32, width: i32, set: bool) {
        Out::rule(self, height, width, set)
    }
    fn special(&mut self, text: &str) {
        Out::special(self, text.as_bytes())
    }
    fn begin_page(&mut self, counts: [i32; 10]) {
        Out::begin_page(self, counts)
    }
    fn end_page(&mut self) {
        Out::end_page(self)
    }
    fn note_push(&mut self, depth: i32) {
        self.max_push = self.max_push.max(depth);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out() -> Out {
        Out::new("", 1000, vec![None])
    }

    /// §610: the size of a `right` follows its operand, and a negative one
    /// is written in two's complement in that many bytes.
    #[test]
    fn a_movement_takes_the_fewest_bytes_it_fits() {
        let mut o = out();
        let start = o.here();
        o.right(100);
        o.right(-200);
        assert_eq!(&o.bytes[start..], &[RIGHT1, 100, RIGHT1 + 1, 0xff, 0x38]);
    }

    /// §607-§613: the second movement of the same size turns the first
    /// into `w1` and is itself `w0`.
    #[test]
    fn a_repeated_movement_is_w0_after_w1() {
        let mut o = out();
        let start = o.here();
        o.right(5);
        o.right(5);
        assert_eq!(&o.bytes[start..], &[RIGHT1 + (Y1 - DOWN1), 5, W0]);
        let mut o = out();
        let start = o.here();
        o.down(7);
        o.down(9);
        o.down(7);
        assert_eq!(&o.bytes[start..], &[Y1, 7, DOWN1, 9, Y0]);
    }

    /// §601: a push that nothing followed is taken back, not popped.
    #[test]
    fn an_empty_push_is_taken_back() {
        let mut o = out();
        let start = o.here();
        let loc = o.push();
        o.pop(loc);
        assert_eq!(o.here(), start);
        let loc = o.push();
        o.right(3);
        o.pop(loc);
        assert_eq!(&o.bytes[start..], &[PUSH, RIGHT1, 3, POP]);
    }
}
