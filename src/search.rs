//! Ranked, typo-tolerant name search over a scan tree.
//!
//! The scoring (fzf-style bonuses, one-typo prefixes, char-class masks) and
//! the query language are adapted from FSearch by Noah Dunnagan
//! (<https://github.com/noahdunnagan/fsearch>, MIT). FSearch scores ASCII
//! bytes in a whole-disk macOS index; this port scores case-folded Unicode
//! names and walks the in-memory tree, so folder names below the current
//! scope count towards a match without building path strings.

use crate::storage::{self, ScanNode};

/// Positive tokens tracked per query (matches `Inherit::best`).
const MAX_TOKENS: usize = 8;
/// Fuzzy words this long forgive one typo.
const TYPO_MIN_LEN: usize = 5;
/// What a typo costs, so clean matches of the same quality rank first.
const TYPO_COST: i32 = 60;

const SCORE_MATCH: i32 = 16;
const GAP_START: i32 = -3;
const GAP_EXT: i32 = -1;
const BONUS_BOUNDARY: i32 = 8;
const BONUS_CAMEL: i32 = 7;
const BONUS_CONSEC: i32 = 4;

#[rustfmt::skip]
const TYPES: &[(&str, &[&str])] = &[
    ("image", &["png", "jpg", "jpeg", "gif", "heic", "heif", "webp", "tiff", "tif", "bmp", "svg", "raw", "cr2", "cr3", "nef", "arw", "dng", "psd", "ico", "avif", "jxl"]),
    ("video", &["mp4", "mov", "m4v", "mkv", "avi", "webm", "wmv", "flv", "mpg", "mpeg", "3gp", "hevc", "m2ts"]),
    ("audio", &["mp3", "m4a", "aac", "wav", "flac", "aiff", "aif", "ogg", "opus", "wma", "mid", "midi"]),
    ("doc", &["pdf", "doc", "docx", "txt", "md", "rtf", "odt", "ods", "odp", "ppt", "pptx", "xls", "xlsx", "csv", "epub", "tex"]),
    ("code", &["rs", "c", "h", "cc", "cpp", "hpp", "cs", "go", "py", "js", "mjs", "cjs", "ts", "tsx", "jsx", "java", "kt", "rb", "php", "swift", "sh", "ps1", "bat", "cmd", "lua", "sql", "html", "css", "scss", "json", "yaml", "yml", "toml", "xml", "vue", "svelte", "zig", "dart", "r", "jl", "glsl", "hlsl", "wgsl", "proto", "graphql"]),
    ("archive", &["zip", "tar", "gz", "tgz", "bz2", "xz", "7z", "rar", "zst", "lz4", "cab"]),
    ("disk", &["iso", "img", "vhd", "vhdx", "vmdk", "qcow2", "wim", "esd"]),
    ("app", &["exe", "msi", "msix", "appx", "dll", "sys"]),
    ("font", &["ttf", "otf", "woff", "woff2", "ttc", "fon"]),
];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Fuzzy,
    Exact,
    Prefix,
    Suffix,
}

#[derive(Clone, Debug)]
struct Token {
    text: Vec<char>,
    mask: u64,
    mode: Mode,
    negate: bool,
    /// Char classes a typo may leave out of a matching name: all but the
    /// first letter's, or none when the token takes no typos.
    loose: u64,
    /// `start_bit` of the first letter when the token takes typos.
    start: u64,
}

impl Token {
    /// Can a name with mask `m` match? Cleanly it has every char class; with
    /// a typo, a word in it starts with this token's first letter and at most
    /// one `loose` class is missing.
    #[inline(always)]
    fn fits(&self, m: u64) -> bool {
        let miss = self.mask & !m;
        (miss == 0)
            | ((((miss & !self.loose) | (miss & miss.wrapping_sub(1))) == 0)
                & (m & self.start != 0))
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    File,
    Dir,
}

#[derive(Clone, Debug, Default)]
pub struct Query {
    tokens: Vec<Token>,
    kind: Option<Kind>,
    exts: Vec<Vec<char>>,
    size: Option<(u64, u64)>,
    paths: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub id: usize,
    pub score: i32,
}

impl Query {
    /// Plain words are fuzzy; `'x` exact, `^x` prefix, `x$` suffix, `!x`
    /// exclude, `"two words"` one token. Filters: `ext:` `type:` `kind:`
    /// `size:` `path:`. A pasted absolute path (`C:\...`) is a `path:`
    /// filter. An unknown `key:` is searched as text.
    pub fn parse(s: &str) -> Result<Query, String> {
        let mut q = Query::default();
        for word in split_words(s) {
            if let [d, b':', b'\\' | b'/', ..] = word.as_bytes()
                && d.is_ascii_alphabetic()
            {
                q.paths.push(normalize_path(&word));
                continue;
            }
            if let Some((k, v)) = word.split_once(':')
                && q.filter(k, v)
                    .map_err(|_| format!("Invalid search filter: {word}"))?
            {
                continue;
            }
            for piece in word
                .split(['/', '\\'])
                .filter(|p| !p.is_empty() && !is_drive(p))
            {
                q.push_token(piece);
            }
        }
        Ok(q)
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
            && self.kind.is_none()
            && self.exts.is_empty()
            && self.size.is_none()
            && self.paths.is_empty()
    }

    pub fn wants_dirs(&self) -> bool {
        self.kind != Some(Kind::File) && self.exts.is_empty()
    }

    fn push_token(&mut self, w: &str) {
        let (mut t, mut negate, mut mode) = (w, false, Mode::Fuzzy);
        if let Some(r) = t.strip_prefix('!') {
            (t, negate, mode) = (r, true, Mode::Exact);
        }
        if let Some(r) = t.strip_prefix('\'') {
            (t, mode) = (r, Mode::Exact);
        } else if let Some(r) = t.strip_prefix('^') {
            (t, mode) = (r, Mode::Prefix);
        } else if let Some(r) = t.strip_suffix('$') {
            (t, mode) = (r, Mode::Suffix);
        }
        if t.is_empty() || (!negate && self.positive().count() >= MAX_TOKENS) {
            return;
        }
        let text: Vec<char> = t.chars().map(fold).collect();
        let mask = text.iter().fold(0, |m, &c| m | char_bit(c));
        let (loose, start) = if takes_typos(&text, mode) {
            (mask & !char_bit(text[0]), start_bit(text[0]))
        } else {
            (0, 0)
        };
        self.tokens.push(Token {
            text,
            mask,
            mode,
            negate,
            loose,
            start,
        });
    }

    fn filter(&mut self, k: &str, v: &str) -> Result<bool, ()> {
        match k.to_ascii_lowercase().as_str() {
            "ext" => {
                for e in v.split(',').map(|e| e.trim_start_matches('.')) {
                    if e.is_empty() {
                        return Err(());
                    }
                    self.exts.push(e.chars().map(fold).collect());
                }
            }
            "type" => {
                for t in v.split(',') {
                    let (_, exts) = TYPES
                        .iter()
                        .find(|(n, _)| n.eq_ignore_ascii_case(t))
                        .ok_or(())?;
                    self.exts.extend(exts.iter().map(|e| e.chars().collect()));
                }
            }
            "kind" => {
                self.kind = Some(match v.to_ascii_lowercase().as_str() {
                    "file" | "f" => Kind::File,
                    "dir" | "folder" | "d" => Kind::Dir,
                    _ => return Err(()),
                })
            }
            "size" => self.size = Some(range(v)?),
            "path" if !v.is_empty() => self.paths.push(normalize_path(v)),
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn positive(&self) -> impl Iterator<Item = &Token> {
        self.tokens.iter().filter(|t| !t.negate)
    }

    /// Every match strictly below `scope`. A name must match at least one
    /// word; the rest may match folders between `scope` and the entry, at
    /// three quarters of their score. Excluded folders prune their subtree.
    ///
    /// Scans run in id order, where a parent always precedes its children,
    /// so one linear pass sees every folder before its contents. Only
    /// folders keep state: their best score per word for descendants.
    /// `masks` holds each entry's name mask (`NameMasks`).
    pub fn search(&self, nodes: &[ScanNode], masks: &[u64], scope: usize) -> Vec<Hit> {
        const OUTSIDE: u32 = u32::MAX;
        let mut hits = Vec::new();
        if scope >= nodes.len() || masks.len() != nodes.len() {
            return hits;
        }
        let positive: Vec<&Token> = self.positive().collect();
        let negative: Vec<&Token> = self.tokens.iter().filter(|t| t.negate).collect();
        let all = (1u32 << positive.len()) - 1;
        let wants_dirs = self.wants_dirs();
        let mut name = Name::default();
        // Folder id -> index into `folders`; OUTSIDE when not below scope
        // or excluded. Ids before the scope cannot descend from it.
        let mut slot = vec![OUTSIDE; nodes.len() - scope];
        let mut folders = vec![[None; MAX_TOKENS]];
        slot[0] = 0;
        // For `path:` filters, each folder's normalized path, parallel to
        // `folders`; entries append their own name to their parent's.
        let mut folder_paths = Vec::new();
        if !self.paths.is_empty() {
            folder_paths.push(normalize_path(
                &storage::node_path(nodes, scope).to_string_lossy(),
            ));
        }
        let mut path = String::new();
        for (id, node) in nodes.iter().enumerate().skip(scope + 1) {
            let Some(parent) = node.parent.and_then(|p| p.checked_sub(scope)) else {
                continue;
            };
            let Some(&from) = slot.get(parent).filter(|&&s| s != OUTSIDE) else {
                continue;
            };
            let inherited = folders[from as usize];
            let descend = node.is_dir && !node.children.is_empty();
            let candidate = if node.is_dir {
                wants_dirs
            } else {
                self.kind != Some(Kind::Dir)
            } && self.cheap_filters_ok(node);
            // Entries that fail a filter need no scoring unless their name
            // counts for descendants.
            if !descend && !candidate {
                continue;
            }
            // Most names lack a letter of every word: reject on the cached
            // mask before decoding and folding them.
            let mut own = [None; MAX_TOKENS];
            let m = masks[id];
            if self.tokens.iter().any(|t| t.fits(m)) {
                name.load(&node.name);
                if negative.iter().any(|t| token_matches(&name, t)) {
                    continue;
                }
                for (o, tok) in own.iter_mut().zip(&positive) {
                    *o = token_score(&name, tok);
                }
            }
            let (mut got, mut inherit, mut score) = (0u32, 0u32, 0i32);
            for (t, own) in own.iter().enumerate().take(positive.len()) {
                if let Some(s) = *own {
                    got |= 1 << t;
                    score += s;
                } else if let Some(s) = inherited[t] {
                    inherit |= 1 << t;
                    score += s * 3 / 4;
                }
            }
            let matched =
                candidate && (positive.is_empty() || (got != 0 && (got | inherit) == all));
            // Building the path costs most, so it runs last and only when needed.
            if !self.paths.is_empty() && (matched || descend) {
                path.clear();
                path.push_str(&folder_paths[from as usize]);
                if !path.ends_with('\\') {
                    path.push('\\');
                }
                path.push_str(&node.name.to_lowercase());
            }
            if matched && self.paths.iter().all(|p| path.contains(p.as_str())) {
                hits.push(Hit { id, score });
            }
            if descend {
                let mut best = inherited;
                for (b, o) in best.iter_mut().zip(own) {
                    *b = (*b).max(o);
                }
                slot[id - scope] = folders.len() as u32;
                folders.push(best);
                if !self.paths.is_empty() {
                    folder_paths.push(path.clone());
                }
            }
        }
        hits
    }

    fn cheap_filters_ok(&self, node: &ScanNode) -> bool {
        self.size
            .is_none_or(|(lo, hi)| (lo..=hi).contains(&node.allocated))
            && (self.exts.is_empty() || (!node.is_dir && ext_ok(&node.name, &self.exts)))
    }
}

/// Every entry's name mask, computed once per scan: names never change after
/// a scan delivers them, so a keystroke reads eight bytes per entry instead
/// of decoding every name. Extends as a running scan adds entries.
#[derive(Default)]
pub struct NameMasks {
    /// Scan generation and tree identity the masks were computed for.
    key: (u64, usize),
    masks: Vec<u64>,
}

impl NameMasks {
    pub fn update(&mut self, nodes: &std::sync::Arc<Vec<ScanNode>>, generation: u64) -> &[u64] {
        let key = (generation, std::sync::Arc::as_ptr(nodes) as usize);
        if self.key != key || self.masks.len() > nodes.len() {
            self.key = key;
            self.masks.clear();
        }
        let mut name = Name::default();
        let start = self.masks.len();
        self.masks.extend(nodes[start..].iter().map(|n| {
            ascii_mask(&n.name).unwrap_or_else(|| {
                name.load(&n.name);
                name.mask
            })
        }));
        &self.masks
    }
}

/// A name's original and case-folded chars plus its char-class mask, in
/// buffers reused across the whole tree walk.
#[derive(Default)]
struct Name {
    raw: Vec<char>,
    folded: Vec<char>,
    mask: u64,
}

impl Name {
    fn load(&mut self, s: &str) {
        self.raw.clear();
        self.folded.clear();
        let mut mask = 0;
        let mut word_start = true;
        for c in s.chars() {
            let f = fold(c);
            mask |= char_bit(f);
            // A leading dot doesn't start the first word: ".gitignore".
            if word_start && !(c == '.' && self.raw.is_empty() && s.len() > 1) {
                mask |= start_bit(f);
                word_start = false;
            }
            word_start |= c == ' ';
            self.raw.push(c);
            self.folded.push(f);
        }
        self.mask = mask;
    }

    fn len(&self) -> i32 {
        self.raw.len().min(80) as i32
    }
}

/// `Name::mask` straight from the bytes of an ASCII name; None otherwise,
/// since Unicode case folding can move a char into another class.
#[inline]
fn ascii_mask(s: &str) -> Option<u64> {
    const BITS: [u64; 128] = {
        let mut table = [0; 128];
        let mut b = 0;
        while b < 128 {
            table[b] = char_bit(b as u8 as char);
            b += 1;
        }
        table
    };
    if !s.is_ascii() {
        return None;
    }
    let bytes = s.as_bytes();
    let mut mask = bytes.iter().fold(0, |m, &b| m | BITS[b as usize]);
    let off = (bytes.len() > 1 && bytes[0] == b'.') as usize;
    if let Some(&b) = bytes.get(off) {
        mask |= start_bit(b as char);
    }
    if mask & char_bit(' ') != 0 {
        for (i, _) in bytes.iter().enumerate().filter(|(_, b)| **b == b' ') {
            if let Some(&b) = bytes.get(i + 1) {
                mask |= start_bit(b as char);
            }
        }
    }
    Some(mask)
}

#[cfg(test)]
fn named(s: &str) -> Name {
    let mut n = Name::default();
    n.load(s);
    n
}

fn takes_typos(text: &[char], mode: Mode) -> bool {
    mode == Mode::Fuzzy && text.len() >= TYPO_MIN_LEN
}

/// Lowercase with Windows separators, so `path:a/b` finds `A\B`.
fn normalize_path(p: &str) -> String {
    p.to_lowercase().replace('/', "\\")
}

fn is_drive(p: &str) -> bool {
    let b = p.as_bytes();
    b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

/// Split on spaces, keeping "double quoted" runs together.
fn split_words(s: &str) -> Vec<String> {
    let (mut out, mut cur, mut quoted) = (Vec::new(), String::new(), false);
    for c in s.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// `>x`, `>=x`, `<x`, `<=x`, `a..b` or exactly `x`, in bytes.
fn range(v: &str) -> Result<(u64, u64), ()> {
    if let Some(r) = v.strip_prefix(">=") {
        return Ok((parse_size(r)?, u64::MAX));
    }
    if let Some(r) = v.strip_prefix('>') {
        return Ok((parse_size(r)?.saturating_add(1), u64::MAX));
    }
    if let Some(r) = v.strip_prefix("<=") {
        return Ok((0, parse_size(r)?));
    }
    if let Some(r) = v.strip_prefix('<') {
        return Ok((0, parse_size(r)?.checked_sub(1).ok_or(())?));
    }
    if let Some((a, b)) = v.split_once("..") {
        let (a, b) = (parse_size(a)?, parse_size(b)?);
        return if a <= b { Ok((a, b)) } else { Err(()) };
    }
    let x = parse_size(v)?;
    Ok((x, x))
}

/// Units as the rest of the app labels them: `k` `m` `g` `t` and the IEC
/// `kib`..`tib` are binary, matching the table; SI `kb`..`tb` are decimal.
fn parse_size(s: &str) -> Result<u64, ()> {
    let i = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let n: f64 = s[..i].parse().map_err(|_| ())?;
    let unit: f64 = match s[i..].to_ascii_lowercase().as_str() {
        "" | "b" => 1.,
        "k" | "kib" => 1024f64,
        "m" | "mib" => 1024f64.powi(2),
        "g" | "gib" => 1024f64.powi(3),
        "t" | "tib" => 1024f64.powi(4),
        "kb" => 1e3,
        "mb" => 1e6,
        "gb" => 1e9,
        "tb" => 1e12,
        _ => return Err(()),
    };
    let bytes = (n * unit).round();
    if bytes.is_finite() && bytes >= 0. {
        Ok(bytes.min(u64::MAX as f64) as u64)
    } else {
        Err(())
    }
}

fn ext_ok(name: &str, exts: &[Vec<char>]) -> bool {
    let Some((_, ext)) = name.rsplit_once('.') else {
        return false;
    };
    exts.iter()
        .any(|e| e.len() == ext.chars().count() && ext.chars().map(fold).eq(e.iter().copied()))
}

/// Simple case folding; a char whose lowercase is several chars stays as is.
#[inline(always)]
fn fold(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_lowercase();
    }
    let mut lower = c.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

#[inline]
const fn char_bit(c: char) -> u64 {
    match c {
        'a'..='z' => 1 << (c as u32 - 'a' as u32),
        'A'..='Z' => 1 << (c as u32 - 'A' as u32),
        '0'..='9' => 1 << (26 + c as u32 - '0' as u32),
        '.' => 1 << 36,
        '-' | '_' => 1 << 37,
        ' ' => 1 << 38,
        '\u{80}'.. => 1 << 39,
        _ => 1 << 40,
    }
}

/// The mask bit for a word starting with `c` (bits 41..64).
#[inline]
fn start_bit(c: char) -> u64 {
    1 << (41 + fold(c) as u32 % 23)
}

#[derive(Clone, Copy, PartialEq)]
enum Class {
    Lower,
    Upper,
    Digit,
    Delim,
    Other,
}

#[inline(always)]
fn class(c: char) -> Class {
    match c {
        ' ' | '_' | '-' | '.' | '/' | '\\' | '(' | ')' | '[' | ']' | ',' | '+' | '@' => {
            Class::Delim
        }
        c if c.is_ascii_digit() => Class::Digit,
        c if c.is_uppercase() => Class::Upper,
        c if c.is_alphabetic() => Class::Lower,
        _ => Class::Other,
    }
}

#[inline(always)]
fn bonus(prev: Class, cur: Class) -> i32 {
    match (prev, cur) {
        (Class::Delim, c) if c != Class::Delim => BONUS_BOUNDARY,
        (Class::Lower, Class::Upper) | (Class::Lower | Class::Upper, Class::Digit) => BONUS_CAMEL,
        _ => 0,
    }
}

fn find(s: &[char], c: char) -> Option<usize> {
    s.iter().position(|&x| x == c)
}

fn rfind(s: &[char], c: char) -> Option<usize> {
    s.iter().rposition(|&x| x == c)
}

/// Where the stem ends: the last dot, unless it is a leading one.
fn stem(name: &Name, off: usize) -> usize {
    rfind(&name.raw, '.')
        .filter(|&p| p > off)
        .unwrap_or(name.raw.len())
}

/// fzf-v1 style: leftmost-ending match, shrunk from the right, then scored
/// with boundary/camel/consecutive bonuses. Whole-name, stem and prefix
/// matches add up to `cap`.
fn fuzzy_score(name: &Name, q: &[char], cap: i32) -> Option<i32> {
    let (raw, folded) = (&name.raw, &name.folded);
    let mut end = 0;
    let mut from = 0;
    for &c in q {
        end = from + find(&folded[from..], c)?;
        from = end + 1;
    }
    let mut start = end + 1;
    for &c in q.iter().rev() {
        start = rfind(&folded[..start], c)?;
    }
    let mut score = 0;
    let (mut at, mut first_bonus) = (start, 0);
    for (k, &c) in q.iter().enumerate() {
        let mut run = false;
        if k > 0 {
            let last = at;
            at = last + 1 + find(&folded[last + 1..=end], c)?;
            run = at == last + 1;
            if !run {
                score += GAP_START + (at - last - 2) as i32 * GAP_EXT;
            }
        }
        let prev = if at == 0 {
            Class::Delim
        } else {
            class(raw[at - 1])
        };
        let mut b = bonus(prev, class(raw[at]));
        if run {
            if b >= BONUS_BOUNDARY && b > first_bonus {
                first_bonus = b;
            }
            b = b.max(first_bonus).max(BONUS_CONSEC);
        } else {
            first_bonus = b;
        }
        score += SCORE_MATCH + if k == 0 { b * 2 } else { b };
    }
    // A leading dot doesn't count: "gitignore" means ".gitignore".
    let off = (raw.len() > 1 && raw[0] == '.') as usize;
    let contiguous = end + 1 - start == q.len();
    let placed = if start == off && contiguous && end + 1 == raw.len() {
        100
    } else if start == off && contiguous && end + 1 == stem(name, off) {
        80
    } else if start == off && contiguous {
        30
    } else {
        0
    };
    Some(score + placed.min(cap) - name.len() / 3)
}

/// Best score for `q` read with one typo at the start of the name or of a
/// space-separated word in it, scored as if typed correctly minus
/// TYPO_COST, and never placed above a prefix.
fn typo_score(name: &Name, q: &[char]) -> Option<i32> {
    let off = (name.raw.len() > 1 && name.raw[0] == '.') as usize;
    let mut best = typo_at(name, off, q);
    if name.mask & char_bit(' ') != 0 {
        for (i, _) in name.raw.iter().enumerate().filter(|(_, c)| **c == ' ') {
            best = best.max(typo_at(name, i + 1, q));
        }
    }
    best
}

fn typo_at(name: &Name, s: usize, q: &[char]) -> Option<i32> {
    let word = name.folded.get(s..)?;
    if word.first() != Some(&q[0]) {
        return None;
    }
    let fixed = word.get(..one_edit_prefix(word, q)?)?;
    Some(fuzzy_score(name, fixed, 30)? - TYPO_COST)
}

/// How long a prefix of `w` the query `q` spells with exactly one edit (a
/// wrong, extra, missing or swapped letter). Digits are never edited:
/// "hat_18" is another file than "hat_98", not a typo of it.
fn one_edit_prefix(w: &[char], q: &[char]) -> Option<usize> {
    let starts =
        |w: &[char], q: &[char]| w.len() >= q.len() && w.iter().zip(q).all(|(a, b)| a == b);
    // The first difference; none means `q` is a clean prefix, not a typo.
    let i = (0..q.len()).find(|&i| i >= w.len() || w[i] != q[i])?;
    if q[i].is_ascii_digit() || w.get(i).is_some_and(char::is_ascii_digit) {
        return None;
    }
    let rest = &q[i + 1..];
    let after = w.get(i + 1..).unwrap_or_default();
    if i + 1 < q.len()
        && i + 1 < w.len()
        && w[i] == q[i + 1]
        && w[i + 1] == q[i]
        && starts(&w[i + 2..], &q[i + 2..])
    {
        return Some(q.len());
    }
    if i < w.len() && starts(after, rest) {
        return Some(q.len());
    }
    if starts(&w[i..], rest) {
        return Some(q.len() - 1);
    }
    (i < w.len() && starts(after, &q[i..])).then_some(q.len() + 1)
}

fn find_slice(hay: &[char], needle: &[char]) -> Option<usize> {
    if needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

fn token_score(name: &Name, t: &Token) -> Option<i32> {
    let m = name.mask;
    let len = name.len();
    let folded = &name.folded;
    match t.mode {
        Mode::Fuzzy if takes_typos(&t.text, t.mode) => {
            if !t.fits(m) {
                return None;
            }
            let clean = if t.mask & !m == 0 {
                fuzzy_score(name, &t.text, 100)
            } else {
                None
            };
            clean.max(if m & t.start != 0 {
                typo_score(name, &t.text)
            } else {
                None
            })
        }
        Mode::Fuzzy => (t.mask & !m == 0)
            .then(|| fuzzy_score(name, &t.text, 100))
            .flatten(),
        Mode::Exact => {
            find_slice(folded, &t.text).map(|p| 40 + if p == 0 { 30 } else { 0 } - len / 3)
        }
        Mode::Prefix => folded.starts_with(&t.text).then(|| 60 - len / 3),
        Mode::Suffix => folded.ends_with(&t.text).then(|| 50 - len / 3),
    }
}

/// Exclusions match exactly (`!x` is always exact), so a stray letter
/// cannot hide a whole folder.
fn token_matches(name: &Name, t: &Token) -> bool {
    token_score(name, t).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(name: &str, q: &str) -> Option<i32> {
        let query = Query::parse(q).unwrap();
        token_score(&named(name), &query.tokens[0])
    }

    /// A tree from `(parent, name, is_dir, logical)`, root first, parents
    /// before children, the way a scan delivers it.
    fn tree(entries: &[(Option<usize>, &str, bool, u64)]) -> Vec<ScanNode> {
        let mut nodes: Vec<ScanNode> = Vec::new();
        for (id, &(parent, name, is_dir, logical)) in entries.iter().enumerate() {
            if let Some(p) = parent {
                nodes[p].children.push(id);
            }
            nodes.push(ScanNode {
                id,
                parent,
                name: name.into(),
                is_dir,
                logical,
                allocated: logical,
                files: (!is_dir) as u64,
                children: vec![],
                note: String::new(),
                incomplete: false,
            });
        }
        nodes
    }

    fn names(nodes: &[ScanNode], q: &str, scope: usize) -> Vec<String> {
        let tree = std::sync::Arc::new(nodes.to_vec());
        let mut cache = NameMasks::default();
        let mut hits = Query::parse(q)
            .unwrap()
            .search(nodes, cache.update(&tree, 0), scope);
        hits.sort_by_key(|h| (std::cmp::Reverse(h.score), h.id));
        hits.into_iter().map(|h| nodes[h.id].name.clone()).collect()
    }

    fn fixture() -> Vec<ScanNode> {
        tree(&[
            (None, r"C:\scan", true, 0),
            (Some(0), "Projects", true, 0),
            (Some(1), "main.rs", false, 2_000),
            (Some(1), "domain.rs", false, 3_000),
            (Some(1), "Übersicht.pdf", false, 9 << 20),
            (Some(0), "Videos", true, 0),
            (Some(5), "holiday main cut.mp4", false, 3 << 30),
            (Some(5), "notes.txt", false, 10),
            (Some(0), "main.rs", false, 50),
        ])
    }

    #[test]
    fn whole_names_and_word_starts_outrank_scattered_letters() {
        let exact = score("main.rs", "main.rs").unwrap();
        let stem = score("main.rs", "main").unwrap();
        let inner = score("domain.rs", "main").unwrap();
        assert!(exact > stem && stem > inner, "{exact} {stem} {inner}");
        assert!(score("MyMainFile.rs", "mmf").unwrap() > score("mammoth_fuel.rs", "mmf").unwrap());
        assert_eq!(score("readme.md", "xyz"), None);
    }

    #[test]
    fn five_letter_words_forgive_one_typo_but_never_digits() {
        for typo in ["mian.rs", "mainn.rs", "man.rs", "maim.rs"] {
            assert!(score("main.rs", typo).is_some(), "{typo}");
        }
        let clean = score("manifest.json", "manif").unwrap();
        let typo = score("manifest.json", "mnaif").unwrap();
        assert!(clean > typo);
        assert_eq!(score("hat_18.png", "hat_98"), None);
        assert_eq!(score("main.rs", "mian"), None, "short words take no typos");
    }

    #[test]
    fn unicode_names_fold_case() {
        assert!(
            Query::parse(r"日本 é:\x").is_ok(),
            "multi-byte words must not split a char"
        );
        assert!(score("Übersicht.pdf", "übersicht").is_some());
        assert!(score("Übersicht.pdf", "UEB").is_none());
        assert!(score("Projects_日本語", "日本").is_some());
        assert!(score("ÉTÉ 2025.jpg", "été").is_some());
    }

    #[test]
    fn token_modes() {
        assert!(score("readme.md", "'adme").is_some());
        assert!(score("readme.md", "'rdme").is_none());
        assert!(score("readme.md", "^read").is_some());
        assert!(score("readme.md", "^adme").is_none());
        assert!(score("readme.md", ".md$").is_some());
        assert!(score("readme.md", "read$").is_none());
    }

    #[test]
    fn ranks_whole_tree_and_inherits_folder_words_below_scope() {
        let nodes = fixture();
        let found = names(&nodes, "main", 0);
        assert_eq!(found[..2], ["main.rs", "main.rs"]);
        assert!(found.contains(&"domain.rs".into()));
        assert!(found.contains(&"holiday main cut.mp4".into()));
        // "projects" is a folder word: only entries inside it match both.
        assert_eq!(names(&nodes, "projects main", 0), ["main.rs", "domain.rs"]);
        // The scope's own name is not part of the query.
        assert_eq!(names(&nodes, "projects main", 1), Vec::<String>::new());
        assert_eq!(names(&nodes, "main", 1), ["main.rs", "domain.rs"]);
    }

    #[test]
    fn exclusions_prune_folders_and_filters_select_attributes() {
        let nodes = fixture();
        assert_eq!(
            names(&nodes, "main !projects", 0),
            ["main.rs", "holiday main cut.mp4"]
        );
        assert_eq!(names(&nodes, "ext:mp4", 0), ["holiday main cut.mp4"]);
        assert_eq!(names(&nodes, "type:doc", 0), ["Übersicht.pdf", "notes.txt"]);
        assert_eq!(names(&nodes, "size:>1gib", 0), ["holiday main cut.mp4"]);
        assert_eq!(names(&nodes, "size:1k..5kb", 0), ["main.rs", "domain.rs"]);
        assert_eq!(names(&nodes, "kind:dir", 0), ["Projects", "Videos"]);
        assert_eq!(
            names(&nodes, r"path:\videos\ kind:file", 0),
            ["holiday main cut.mp4", "notes.txt"]
        );
        assert_eq!(
            names(&nodes, r"path:\videos\ kind:file", 0),
            names(&nodes, "path:/VIDEOS/ kind:file", 0)
        );
        assert_eq!(names(&nodes, r"C:\scan\Projects\main", 0), ["main.rs"]);
        assert_eq!(names(&nodes, r"C:\scan\Projects\ ^d", 0), ["domain.rs"]);
    }

    #[test]
    fn quoted_words_and_query_errors() {
        let nodes = fixture();
        assert_eq!(names(&nodes, "\"main cut\"", 0), ["holiday main cut.mp4"]);
        for bad in ["size:>abc", "size:5..1", "kind:blob", "type:nope", "ext:"] {
            assert_eq!(
                Query::parse(bad).unwrap_err(),
                format!("Invalid search filter: {bad}")
            );
        }
        assert!(Query::parse("   ").unwrap().is_empty());
        assert!(!Query::parse("unknown:value").unwrap().is_empty());
    }

    #[test]
    fn byte_mask_prefilter_agrees_with_folded_names() {
        for s in [
            ".gitignore",
            "a  b",
            "Main.RS",
            "holiday main cut.mp4",
            ".",
            " x",
            "a .b",
            "Z9-_@",
        ] {
            assert_eq!(ascii_mask(s), Some(named(s).mask), "{s:?}");
        }
        assert_eq!(ascii_mask("Übersicht"), None);
    }

    #[test]
    fn size_units_follow_their_prefix() {
        assert_eq!(range("5mib"), Ok((5 << 20, 5 << 20)));
        assert_eq!(range("5M"), Ok((5 << 20, 5 << 20)));
        assert_eq!(range("5mb"), Ok((5_000_000, 5_000_000)));
        assert_eq!(range("1.5k"), Ok((1536, 1536)));
        assert_eq!(range("0.1kb"), Ok((100, 100)));
        assert_eq!(range("<1kib"), Ok((0, 1023)));
        assert_eq!(range(">=2g"), Ok((2 << 30, u64::MAX)));
        assert_eq!(range("<0"), Err(()));
    }

    /// Search speed on a real scan, reported in docs/benchmarks.md. Export a
    /// scan, then run:
    /// `RIGOMETRY_BENCH_SCAN=scan.json cargo test --release bench_search -- --ignored --nocapture`
    #[test]
    #[ignore = "needs a scan export; see docs/benchmarks.md"]
    fn bench_search_on_a_real_scan() {
        use std::time::{Duration, Instant};
        let Ok(path) = std::env::var("RIGOMETRY_BENCH_SCAN") else {
            eprintln!("RIGOMETRY_BENCH_SCAN is not set");
            return;
        };
        #[derive(serde::Deserialize)]
        struct Export {
            nodes: Vec<ScanNode>,
        }
        let file = std::io::BufReader::new(std::fs::File::open(path).unwrap());
        let export: Export = serde_json::from_reader(file).unwrap();
        let nodes = std::sync::Arc::new(export.nodes);
        let started = Instant::now();
        let mut cache = NameMasks::default();
        cache.update(&nodes, 1);
        println!(
            "{} entries; name masks built once in {:?}",
            nodes.len(),
            started.elapsed()
        );
        let masks = cache.update(&nodes, 1);
        println!(
            "| Query | Results | p50 | p95 |
| --- | ---: | ---: | ---: |"
        );
        for q in [
            "docker",
            "gemma gguf",
            "reprot",
            "ext:mkv size:>1g",
            "kind:dir node_modules",
            r"path:\steamapps\ ext:pak",
            "readme !node_modules",
        ] {
            // As the UI does per keystroke: parse, search the tree, rank.
            let mut times: Vec<Duration> = Vec::new();
            let mut results = 0;
            for _ in 0..30 {
                let t = Instant::now();
                let query = Query::parse(q).unwrap();
                let mut hits = query.search(&nodes, masks, 0);
                hits.sort_unstable_by_key(|h| std::cmp::Reverse(h.score));
                times.push(t.elapsed());
                results = hits.len();
            }
            times.sort_unstable();
            let ms = |d: Duration| d.as_secs_f64() * 1000.;
            println!(
                "| `{q}` | {results} | {:.0} ms | {:.0} ms |",
                ms(times[times.len() / 2]),
                ms(times[times.len() * 95 / 100])
            );
        }
    }
}
