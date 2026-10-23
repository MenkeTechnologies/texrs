//! Hyphenation in the paragraph builder's second pass: `tex.web` §891-§918.
//!
//! When the first pass finds no feasible breaks, §866 tries to hyphenate the
//! word after every glue it passes. §894-§899 find the word -- its letters by
//! `\lccode`, in one font, between something that stops it -- §923's
//! patterns (here `crate::linebreak::Hyphenator`) say where it may break,
//! and §903-§918 rebuild it with `reconstitute`, the main loop's ligature and
//! kern machinery run again over the letters, so that a discretionary's
//! pre-break text ends in the font's hyphen character ligatured and kerned as
//! tex sets it, and the post-break text starts as a new line would.

use super::font::Font;
use super::Stomach;
use crate::node::{CharNode, DiscNode, Ligature, Node};

/// §549's `non_char`.
const NON_CHAR: u16 = 256;
const STOP_FLAG: u8 = 128;
const KERN_FLAG: u8 = 128;

/// One `lig_stack` item of §907: a character to be set, holding (when it
/// replaced one) the character node `hu[j+1]` it stood for.
#[derive(Clone, Copy)]
struct Item {
    c: u16,
    lig_ptr: Option<u16>,
}

/// §905-§907's state for one word.
struct Word<'a> {
    font: &'a Font,
    f: usize,
    /// `hu[0..=hn]`: the word's characters, `hu[0]` the one before it.
    hu: Vec<u16>,
    /// `hyf[0..=hn]`: odd where a hyphen may go after letter `j`.
    hyf: Vec<u8>,
    /// `init_list`, `init_lig`, `init_lft` (§903).
    init_list: Vec<u16>,
    init_lig: bool,
    init_lft: bool,
    /// `link(hold_head)`: what `reconstitute` built.
    hold: Vec<Node>,
    hyphen_passed: usize,
}

impl Word<'_> {
    fn char_node(&self, c: u16) -> Node {
        let ch = char::from(c as u8);
        let m = self.font.metrics(ch).unwrap_or_default();
        Node::Char(CharNode {
            font: self.f,
            character: ch,
            width: m.width,
            height: m.height,
            depth: m.depth,
        })
    }

    /// `reconstitute(j,n,bchar,hchar)` (§906-§911): translate `hu[j..]` as far
    /// as the ligature program goes in one cursor sweep, into `hold`, and
    /// return the index of the last character consumed.
    fn reconstitute(&mut self, mut j: usize, n: usize, mut bchar: u16, mut hchar: u16) -> usize {
        self.hyphen_passed = 0;
        self.hold.clear();
        let mut w: crate::node::Scaled = 0;
        let mut ligature_present;
        let mut lft_hit = false;
        let mut rt_hit = false;
        let mut lig_stack: Vec<Item> = Vec::new();
        // §908.
        let mut cur_l = self.hu[j];
        let mut cur_q = 0usize;
        if j == 0 {
            ligature_present = self.init_lig;
            if ligature_present {
                lft_hit = self.init_lft;
            }
            for c in self.init_list.clone() {
                let node = self.char_node(c);
                self.hold.push(node);
            }
        } else {
            ligature_present = false;
            if cur_l < NON_CHAR {
                let node = self.char_node(cur_l);
                self.hold.push(node);
            }
        }
        let set_cur_r = |w: &Word, j: usize, bchar: u16, hchar: u16| -> (u16, u16) {
            let r = if j < n { w.hu[j + 1] } else { bchar };
            let rh = if w.hyf[j] % 2 == 1 { hchar } else { NON_CHAR };
            (r, rh)
        };
        let (mut cur_r, mut cur_rh) = set_cur_r(self, j, bchar, hchar);
        // §910's wrap_lig: the characters linked since `cur_q` become a
        // ligature node when one is being built.
        let wrap = |s: &mut Word,
                    cur_q: usize,
                    cur_l: u16,
                    lp: &mut bool,
                    lft: &mut bool,
                    rt: &mut bool,
                    right: bool,
                    stack_empty: bool| {
            if !*lp {
                return;
            }
            let components: Vec<char> = s
                .hold
                .drain(cur_q..)
                .filter_map(|n| match n {
                    Node::Char(c) => Some(c.character),
                    _ => None,
                })
                .collect();
            let mut lig = Ligature {
                components,
                left_boundary: false,
                right_boundary: false,
            };
            if *lft {
                lig.left_boundary = true;
                *lft = false;
            }
            if right && stack_empty {
                lig.right_boundary = true;
                *rt = false;
            }
            let Node::Char(c) = s.char_node(cur_l) else {
                return;
            };
            s.hold.push(Node::Ligature(c, lig));
            *lp = false;
        };
        let mut budget = 64 * (self.hu.len() + 8);
        'continue_: loop {
            budget -= 1;
            if budget == 0 {
                return j;
            }
            // §909: the lig/kern program of `cur_l`, looking for `test_char`.
            'done: {
                let start = match cur_l == NON_CHAR {
                    true => self.font.lig_start(NON_CHAR),
                    false => self.font.lig_start(cur_l),
                };
                let Some(mut k) = start else {
                    break 'done;
                };
                let test_char = if cur_rh < NON_CHAR { cur_rh } else { cur_r };
                loop {
                    let Some([skip, next, op, rem]) = self.font.lig_word(k) else {
                        break 'done;
                    };
                    if u16::from(next) == test_char && skip <= STOP_FLAG {
                        if cur_rh < NON_CHAR {
                            self.hyphen_passed = j;
                            hchar = NON_CHAR;
                            cur_rh = NON_CHAR;
                            continue 'continue_;
                        }
                        if hchar < NON_CHAR && self.hyf[j] % 2 == 1 {
                            self.hyphen_passed = j;
                            hchar = NON_CHAR;
                        }
                        if op < KERN_FLAG {
                            // §911.
                            if cur_l == NON_CHAR {
                                lft_hit = true;
                            }
                            if j == n && lig_stack.is_empty() {
                                rt_hit = true;
                            }
                            let rem = u16::from(rem);
                            match op {
                                1 | 5 => {
                                    cur_l = rem;
                                    ligature_present = true;
                                }
                                2 | 6 => {
                                    cur_r = rem;
                                    match lig_stack.last_mut() {
                                        Some(top) => top.c = rem,
                                        None => {
                                            let lig_ptr = match j == n {
                                                true => {
                                                    bchar = NON_CHAR;
                                                    None
                                                }
                                                false => Some(self.hu[j + 1]),
                                            };
                                            lig_stack.push(Item { c: rem, lig_ptr });
                                        }
                                    }
                                }
                                3 => {
                                    cur_r = rem;
                                    lig_stack.push(Item {
                                        c: rem,
                                        lig_ptr: None,
                                    });
                                }
                                7 | 11 => {
                                    let empty = lig_stack.is_empty();
                                    wrap(
                                        self,
                                        cur_q,
                                        cur_l,
                                        &mut ligature_present,
                                        &mut lft_hit,
                                        &mut rt_hit,
                                        false,
                                        empty,
                                    );
                                    cur_q = self.hold.len();
                                    cur_l = rem;
                                    ligature_present = true;
                                }
                                _ => {
                                    cur_l = rem;
                                    ligature_present = true;
                                    if let Some(top) = lig_stack.pop() {
                                        // pop_lig_stack
                                        if let Some(c) = top.lig_ptr {
                                            let node = self.char_node(c);
                                            self.hold.push(node);
                                            j += 1;
                                        }
                                        match lig_stack.last() {
                                            None => {
                                                (cur_r, cur_rh) = set_cur_r(self, j, bchar, hchar)
                                            }
                                            Some(t) => cur_r = t.c,
                                        }
                                    } else if j == n {
                                        break 'done;
                                    } else {
                                        let node = self.char_node(cur_r);
                                        self.hold.push(node);
                                        j += 1;
                                        (cur_r, cur_rh) = set_cur_r(self, j, bchar, hchar);
                                    }
                                }
                            }
                            if op > 4 && op != 7 {
                                break 'done;
                            }
                            continue 'continue_;
                        }
                        w = self.font.kern(op, rem);
                        break 'done;
                    }
                    if skip >= STOP_FLAG {
                        if cur_rh == NON_CHAR {
                            break 'done;
                        }
                        cur_rh = NON_CHAR;
                        continue 'continue_;
                    }
                    k += usize::from(skip) + 1;
                }
            }
            // §910.
            let empty = lig_stack.is_empty();
            let right = rt_hit;
            wrap(
                self,
                cur_q,
                cur_l,
                &mut ligature_present,
                &mut lft_hit,
                &mut rt_hit,
                right,
                empty,
            );
            if w != 0 {
                self.hold.push(Node::Kern {
                    width: w,
                    explicit: false,
                });
                w = 0;
            }
            if let Some(top) = lig_stack.pop() {
                cur_q = self.hold.len();
                cur_l = top.c;
                ligature_present = true;
                if let Some(c) = top.lig_ptr {
                    let node = self.char_node(c);
                    self.hold.push(node);
                    j += 1;
                }
                match lig_stack.last() {
                    None => (cur_r, cur_rh) = set_cur_r(self, j, bchar, hchar),
                    Some(t) => cur_r = t.c,
                }
                continue 'continue_;
            }
            return j;
        }
    }
}

/// `set_lc_code` (§897): the `\lccode` of `c`; INITEX's for the letters
/// unless the document has said otherwise.
fn lc_code(s: &Stomach, c: char) -> u16 {
    match s.lc_codes.get(&c) {
        Some(v) => *v as u16,
        None if c.is_ascii_lowercase() => c as u16,
        None if c.is_ascii_uppercase() => c.to_ascii_lowercase() as u16,
        None => 0,
    }
}

impl Stomach {
    /// §894: "Try to hyphenate the following word" after the glue at
    /// `cur_p`, replacing its nodes by the reconstituted word with
    /// discretionaries in it.
    pub fn hyphenate_after(&mut self, list: &mut Vec<Node>, cur_p: usize) {
        let Some(hyph) = self.hyphenator() else {
            return;
        };
        let l_hyf = self.int_par("lefthyphenmin").clamp(1, 63) as usize;
        let r_hyf = self.int_par("righthyphenmin").clamp(1, 63) as usize;
        if l_hyf + r_hyf > 63 {
            return;
        }
        let uc_hyph = self.int_par("uchyph");
        // §896: skip to node `ha`.
        let mut prev_s = cur_p;
        let mut s = cur_p + 1;
        let (hf, first) = loop {
            let Some(node) = list.get(s) else {
                return;
            };
            let (c, f) = match node {
                Node::Char(c) => (c.character, c.font),
                Node::Ligature(c, lig) => match lig.components.first() {
                    None => {
                        prev_s = s;
                        s += 1;
                        continue;
                    }
                    Some(&k) => (k, c.font),
                },
                Node::Kern { explicit: false, .. } | Node::Whatsit(_) | Node::Out(_) => {
                    prev_s = s;
                    s += 1;
                    continue;
                }
                _ => return,
            };
            let lc = lc_code(self, c);
            if lc != 0 {
                if lc == c as u16 || uc_hyph > 0 {
                    break (f, s);
                }
                return;
            }
            prev_s = s;
            s += 1;
        };
        let font = self.font(hf).clone();
        let hyf_char = font.hyphen_char;
        if !(0..=255).contains(&hyf_char) {
            return;
        }
        let ha = prev_s;
        // §897: the letters, into `hu` and `hc`.
        let mut hu: Vec<u16> = vec![0];
        let mut hc: Vec<u8> = Vec::new();
        let mut hb = first;
        let mut hyf_bchar = NON_CHAR;
        s = first;
        loop {
            match list.get(s) {
                Some(Node::Char(c)) => {
                    if c.font != hf {
                        break;
                    }
                    // `hyf_bchar:=character(s)` BEFORE the letter test: when
                    // this character ends the word, it is the right
                    // neighbour the word's last letter is reconstituted
                    // against (§903's `bchar`), so the kern or ligature
                    // between them -- cmr10's `y.` -- comes back.
                    hyf_bchar = c.character as u16;
                    let lc = lc_code(self, c.character);
                    if lc == 0 || hu.len() > 63 {
                        break;
                    }
                    hb = s;
                    hu.push(c.character as u16);
                    hc.push(lc as u8);
                    hyf_bchar = NON_CHAR;
                }
                // §898.
                Some(Node::Ligature(c, lig)) => {
                    if c.font != hf {
                        break;
                    }
                    let mut letters = Vec::new();
                    let mut ok = true;
                    for &k in &lig.components {
                        let lc = lc_code(self, k);
                        if lc == 0 || hu.len() + letters.len() > 63 {
                            ok = false;
                            break;
                        }
                        letters.push((k as u16, lc as u8));
                    }
                    if !ok {
                        break;
                    }
                    for (k, lc) in letters {
                        hu.push(k);
                        hc.push(lc);
                    }
                    hb = s;
                    hyf_bchar = match lig.right_boundary {
                        true => font.bchar_code(),
                        false => NON_CHAR,
                    };
                }
                Some(Node::Kern { explicit: false, .. }) => {
                    hb = s;
                    hyf_bchar = font.bchar_code();
                }
                _ => break,
            }
            s += 1;
        }
        let hn = hu.len() - 1;
        // §899: what follows the word must permit hyphenation.
        if hn < l_hyf + r_hyf {
            return;
        }
        let mut t = s;
        loop {
            match list.get(t) {
                None => break,
                Some(Node::Char(_)) | Some(Node::Ligature(..)) => {}
                Some(Node::Kern { explicit: false, .. }) => {}
                Some(Node::Kern { .. }) => break,
                Some(
                    Node::Whatsit(_)
                    | Node::Out(_)
                    | Node::Glue(_)
                    | Node::Penalty(_)
                    | Node::Ins(_)
                    | Node::Adjust(_)
                    | Node::Mark(_),
                ) => break,
                Some(_) => return,
            }
            t += 1;
        }
        // §923: the patterns (and exceptions) say where.
        let word: String = hc.iter().map(|&b| b as char).collect();
        let mut hyf = vec![0u8; hn + 2];
        for at in hyph.points(&word) {
            if at >= l_hyf && at <= hn.saturating_sub(r_hyf) {
                hyf[at] = 1;
            }
        }
        // §902.
        if !(l_hyf..=hn.saturating_sub(r_hyf)).any(|j| hyf[j] % 2 == 1) {
            return;
        }
        self.reconstitute_word(list, cur_p, ha, hb, hf, &font, hu, hyf, hyf_char as u16, hyf_bchar);
    }

    /// §903 and §913-§918: replace `ha..hb` by the word rebuilt, with the
    /// discretionary hyphens in it.
    #[allow(clippy::too_many_arguments)]
    fn reconstitute_word(
        &mut self,
        list: &mut Vec<Node>,
        cur_p: usize,
        ha: usize,
        hb: usize,
        hf: usize,
        font: &Font,
        mut hu: Vec<u16>,
        hyf: Vec<u8>,
        hyf_char: u16,
        hyf_bchar: u16,
    ) {
        let hn = hu.len() - 1;
        let bchar = hyf_bchar;
        let mut w = Word {
            font,
            f: hf,
            hu: Vec::new(),
            hyf,
            init_list: Vec::new(),
            init_lig: false,
            init_lft: false,
            hold: Vec::new(),
            hyphen_passed: 0,
        };
        // §903: where the rebuilt nodes go (`s`), and where they start (`j`).
        let found2 = |hu: &mut Vec<u16>| -> (usize, usize) {
            hu[0] = NON_CHAR;
            (ha, 0)
        };
        let (keep_through, mut j) = match &list[ha] {
            Node::Char(c) if c.font == hf => {
                w.init_list = vec![c.character as u16];
                hu[0] = c.character as u16;
                (ha - 1, 0)
            }
            Node::Char(_) => found2(&mut hu),
            Node::Ligature(c, lig) if c.font == hf => {
                w.init_list = lig.components.iter().map(|&k| k as u16).collect();
                w.init_lig = true;
                w.init_lft = lig.left_boundary;
                hu[0] = c.character as u16;
                if w.init_list.is_empty() && w.init_lft {
                    hu[0] = NON_CHAR;
                    w.init_lig = false;
                }
                (ha - 1, 0)
            }
            Node::Ligature(..) => found2(&mut hu),
            _ => {
                // No punctuation found; look for the left boundary.
                if let Some(Node::Ligature(_, lig)) = list.get(ha + 1) {
                    if lig.left_boundary {
                        let (k, j0) = found2(&mut hu);
                        let _ = cur_p;
                        return self.finish_word(list, k, hb, w, hu, j0, hn, bchar, hyf_char);
                    }
                }
                (ha, 1)
            }
        };
        let _ = &mut j;
        self.finish_word(list, keep_through, hb, w, hu, j, hn, bchar, hyf_char);
    }

    /// §913-§918 proper, splicing the result in after `keep_through`.
    #[allow(clippy::too_many_arguments)]
    fn finish_word(
        &mut self,
        list: &mut Vec<Node>,
        keep_through: usize,
        hb: usize,
        mut w: Word,
        hu: Vec<u16>,
        mut j: usize,
        hn: usize,
        bchar: u16,
        hyf_char: u16,
    ) {
        w.hu = hu;
        let font = w.font;
        let mut out: Vec<Node> = Vec::new();
        loop {
            // §913.
            let mut l = j;
            j = w.reconstitute(j, hn, bchar, hyf_char) + 1;
            if w.hyphen_passed == 0 {
                out.append(&mut w.hold);
                if w.hyf[j - 1] % 2 == 1 {
                    l = j;
                    w.hyphen_passed = j - 1;
                    w.hold.clear();
                }
            }
            if w.hyphen_passed > 0 {
                // §914.
                loop {
                    let mut major: Vec<Node> = std::mem::take(&mut w.hold);
                    let mut i = w.hyphen_passed;
                    w.hyf[i] = 0;
                    // §915: the pre-break text, `hu[l..i]` and a hyphen.
                    let mut pre: Vec<Node> = Vec::new();
                    let hyf_node = font.metrics(char::from(hyf_char as u8)).is_some();
                    let mut saved = 0u16;
                    if hyf_node {
                        i += 1;
                        saved = w.hu[i];
                        w.hu[i] = hyf_char;
                    }
                    while l <= i {
                        l = w.reconstitute(l, i, font.bchar_code(), NON_CHAR) + 1;
                        pre.append(&mut w.hold);
                    }
                    if hyf_node {
                        w.hu[i] = saved;
                        l = i;
                        i -= 1;
                    }
                    let _ = i;
                    // §916: the post-break text, and the replacement grown
                    // until the two branches agree again.
                    let mut post: Vec<Node> = Vec::new();
                    let mut c_loc = 0usize;
                    let mut c = 0u16;
                    if font.lig_start(NON_CHAR).is_some() {
                        l -= 1;
                        c = w.hu[l];
                        c_loc = l;
                        w.hu[l] = NON_CHAR;
                    }
                    while l < j {
                        loop {
                            l = w.reconstitute(l, hn, bchar, NON_CHAR) + 1;
                            if c_loc > 0 {
                                w.hu[c_loc] = c;
                                c_loc = 0;
                            }
                            post.append(&mut w.hold);
                            if l >= j {
                                break;
                            }
                        }
                        while l > j {
                            // §917.
                            j = w.reconstitute(j, hn, bchar, NON_CHAR) + 1;
                            major.append(&mut w.hold);
                        }
                    }
                    // §918.
                    let r_count = major.len();
                    if r_count <= 127 {
                        out.push(Node::Disc(DiscNode {
                            pre_break: pre,
                            post_break: post,
                            replace_count: r_count,
                        }));
                    }
                    out.append(&mut major);
                    w.hyphen_passed = j - 1;
                    w.hold.clear();
                    if w.hyf[j - 1] % 2 == 0 {
                        break;
                    }
                }
            }
            if j > hn {
                break;
            }
        }
        // `link(s):=q`.
        let tail = list.split_off(hb + 1);
        list.truncate(keep_through + 1);
        list.extend(out);
        list.extend(tail);
    }

    /// The patterns as a hyphenator, built the first time a paragraph needs
    /// one.
    fn hyphenator(&mut self) -> Option<std::rc::Rc<crate::linebreak::Hyphenator>> {
        if self.hyphenation_source.is_empty() {
            return None;
        }
        if self.hyphenator.is_none() {
            let h = crate::linebreak::Hyphenator::from_source(&self.hyphenation_source);
            self.hyphenator = Some(std::rc::Rc::new(h));
        }
        self.hyphenator.clone()
    }
}

impl Stomach {
    /// Every word after a glue node of the paragraph, hyphenated (§866's
    /// "Try to hyphenate the following word", for the whole pass): after
    /// glue where `auto_breaking` holds, which is not inside a formula.
    pub fn hyphenate_all(&mut self, list: &mut Vec<Node>) {
        let mut auto_breaking = true;
        let mut i = 0;
        while i < list.len() {
            match list[i] {
                Node::Math(_) => auto_breaking = false,
                Node::MathOff(_) => auto_breaking = true,
                Node::Glue(_) if auto_breaking => self.hyphenate_after(list, i),
                _ => {}
            }
            i += 1;
        }
    }
}
