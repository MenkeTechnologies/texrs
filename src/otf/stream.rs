//! The byte reader the ConTeXt fontloader reads a font with
//! (`utilities.files` over LuaTeX's `fio` library): big-endian integers at a
//! position in the WHOLE file, which every table reader sets itself.
//!
//! Reading past the end gives zero rather than failing. `fio` returns `nil`
//! there and the Lua around it then fails on arithmetic, inside the `xpcall`
//! `loadfont` wraps everything in (font-otr.lua:2346), so such a font does not
//! load at all; a font damaged that way is reported by the caller from
//! [`Stream::overrun`], which says whether any read fell off the end.

#[derive(Clone)]
pub struct Stream<'a> {
    data: &'a [u8],
    pos: usize,
    overrun: bool,
}

impl<'a> Stream<'a> {
    pub fn new(data: &'a [u8]) -> Stream<'a> {
        Stream {
            data,
            pos: 0,
            overrun: false,
        }
    }

    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Whether any read so far ran off the end of the file.
    pub fn overrun(&self) -> bool {
        self.overrun
    }

    /// `setposition(f, n)`: zero based, as `fio` counts.
    pub fn set(&mut self, pos: usize) {
        self.pos = pos;
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    fn byte(&mut self) -> u8 {
        let b = match self.data.get(self.pos) {
            Some(&b) => b,
            None => {
                self.overrun = true;
                0
            }
        };
        self.pos += 1;
        b
    }

    /// `readcardinal1`.
    pub fn u8(&mut self) -> u32 {
        self.byte() as u32
    }

    /// `readcardinal2`.
    pub fn u16(&mut self) -> u32 {
        (self.u8() << 8) | self.u8()
    }

    /// `readcardinal3`.
    pub fn u24(&mut self) -> u32 {
        (self.u16() << 8) | self.u8()
    }

    /// `readcardinal4`.
    pub fn u32(&mut self) -> u32 {
        (self.u16() << 16) | self.u16()
    }

    /// `readinteger2`.
    pub fn i16(&mut self) -> i32 {
        self.u16() as u16 as i16 as i32
    }

    /// `readinteger4`.
    pub fn i32(&mut self) -> i64 {
        self.u32() as i32 as i64
    }

    /// `readfixed4`: a signed 16.16 number.
    pub fn fixed(&mut self) -> f64 {
        self.i32() as f64 / 65536.0
    }

    /// `read2dot14`: a signed 2.14 number.
    pub fn f2dot14(&mut self) -> f64 {
        self.i16() as f64 / 16384.0
    }

    /// `readstring(f, n)`: `n` bytes, as they are.
    pub fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.byte()).collect()
    }

    /// `skipshort(f, n)`.
    pub fn skip_u16(&mut self, n: usize) {
        self.pos += 2 * n;
    }

    /// `readcardinaltable(f, n, ushort)`.
    pub fn u16s(&mut self, n: usize) -> Vec<u32> {
        (0..n).map(|_| self.u16()).collect()
    }

    /// `readcardinaltable(f, n, ulong)`.
    pub fn u32s(&mut self, n: usize) -> Vec<u32> {
        (0..n).map(|_| self.u32()).collect()
    }
}
