//! Syntax highlighting for fenced code (MDEDIT-190).
//!
//! The one entry point is [`highlight`]: an info string and a fence's body in, a list of
//! `(ByteSpan, Token)` out. It decorates and never rewrites, so a caller that draws the spans and
//! leaves the gaps plain has changed no byte of the buffer.
//!
//! **The language is chosen by the info string and by nothing else.** A fence with no info string is
//! never highlighted, and nothing reads the code to guess what it is — content sniffing is the trust
//! boundary the 2023 Notepad++ CVEs were all in. A JSON body under ```` ```text ```` is plain text.
//!
//! **The backend is `fancy-regex`, not Oniguruma.** A fence's contents are untrusted file contents
//! and Oniguruma is C. See `docs/decisions/languages.md` for the numbers and for the syntaxes that
//! `fancy-regex` cannot run.
//!
//! **Loading is lazy.** The syntax set is deserialised on the first call that names a language, not
//! at startup, and [`is_loaded`] says whether that has happened.
//!
//! **This is expensive and is meant to run off the thread that draws** (MDEDIT-190): about 4 us a byte
//! in a release build, and 54 ms to load the syntax set. The editor calls it from one worker thread.
//!
//! Highlighting is best effort. A line the regex engine refuses ends the highlighting there and the
//! rest of the fence draws plain; nothing in here panics on what a file contains. A fence is bounded
//! by [`MAX_LINE_BYTES`] and by [`MAX_WORK`], and past either the rest draws plain.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, PoisonError};

use mdedit_language::Token;
use mdedit_offsets::{ByteOffset, ByteSpan};
use syntect::dumps::from_uncompressed_data;
use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxSet};

/// Past this many bytes a fence draws plain. Measured at about 4 us a byte in release (MDEDIT-190), so
/// this keeps one job on the worker thread under about half a second.
pub const MAX_BODY_BYTES: usize = 128 * 1024;

/// Past this many bytes a single line ends the highlighting, and the rest of the fence draws plain.
/// The regex engine's cost on one line grows faster than its length: a 4 KB line took about 280 ms
/// in PowerShell and 401 ms in `rust` (MDEDIT-202), so 1 KB keeps the worst line near 16 times
/// cheaper.
pub const MAX_LINE_BYTES: usize = 1024;

/// The most work one fence may cost, where a line costs its length squared: a line's cost is
/// quadratic in its length (MDEDIT-202), so a fence is charged the sum of each line's `len²`. This
/// fits 128 KiB of lines averaging 256 bytes, and a hostile fence of 1 KiB lines stops after about 32
/// of them. There is no clock, so the result is the same on every machine.
pub const MAX_WORK: u64 = 32 * 1024 * 1024;

/// How many distinct info tokens are remembered before the table is cleared. A document chooses
/// them, so the table must not grow with it.
const MAX_REMEMBERED_TOKENS: usize = 256;

/// The longest info token worth looking up; no syntax name or extension is longer.
const MAX_TOKEN_BYTES: usize = 64;

/// Scope prefixes and the token they are drawn as. The longest matching prefix wins.
///
/// Every colour here is one the JSON and YAML lexers already use, so a fence reads in the theme's
/// own colours rather than a second palette; an unmatched scope gets no span and draws plain.
const SCOPES: &[(&str, Token)] = &[
    ("comment", Token::Comment),
    ("string", Token::String),
    ("constant.numeric", Token::Number),
    ("constant.language", Token::Literal),
    ("constant.character", Token::Literal),
    ("support.constant", Token::Literal),
    ("keyword", Token::Key),
    ("storage", Token::Key),
    ("entity.name", Token::Tag),
    ("support.function", Token::Tag),
    ("support.type", Token::Tag),
    ("entity.other.attribute-name", Token::Tag),
    ("punctuation", Token::Punctuation),
];

/// Info strings that name a language under another word. TypeScript has no syntax of its own: the only
/// maintained ones are `version: 2` and syntect 5.3.0 cannot load them, so it is drawn as JavaScript
/// until syntect 6.0 (MDEDIT-201).
const ALIASES: &[(&str, &str)] = &[
    ("ts", "js"),
    ("tsx", "js"),
    ("typescript", "js"),
    ("jsx", "js"),
    ("mts", "js"),
    ("cts", "js"),
    ("pwsh", "powershell"),
    ("ps", "powershell"),
    ("docker", "dockerfile"),
];

static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
static MEMO: Mutex<Option<Memo>> = Mutex::new(None);

#[derive(Debug, Default)]
struct Memo {
    syntax_by_token: HashMap<String, Option<usize>>,
    token_by_scope: HashMap<Scope, Option<Token>>,
}

/// The merged set `build.rs` dumped: syntect's bundled syntaxes plus the vendored ones.
#[allow(clippy::expect_used)]
fn syntaxes() -> &'static SyntaxSet {
    SYNTAXES.get_or_init(|| {
        from_uncompressed_data(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/syntaxes.packdump"
        )))
        .expect("the build script wrote a valid dump")
    })
}

/// Whether the syntax set has been loaded yet. Exists so a test can prove laziness.
#[must_use]
pub fn is_loaded() -> bool {
    SYNTAXES.get().is_some()
}

/// The highlight spans of `code`, a fence's body, for the language `info` names.
///
/// Spans are relative to `code`, sorted, non-overlapping and on character boundaries. Text with no
/// colour of its own has no span. `None` means `info` names no known language — including when it
/// is empty, in which case nothing is loaded. A fence is coloured up to the first line over
/// [`MAX_LINE_BYTES`], or the first line that would take its total cost past [`MAX_WORK`], and is
/// plain after that.
///
/// ```
/// use mdedit_highlight::highlight;
///
/// assert!(highlight("", "let x = 1;").is_none());
/// assert!(highlight("rust", "let x = 1;\n").is_some());
/// ```
#[must_use]
pub fn highlight(info: &str, code: &str) -> Option<Vec<(ByteSpan, Token)>> {
    highlight_within(info, code, MAX_WORK)
}

/// [`highlight`] with the work budget given, so a test can choose it.
#[doc(hidden)]
#[must_use]
pub fn highlight_within(info: &str, code: &str, max_work: u64) -> Option<Vec<(ByteSpan, Token)>> {
    let word = info.split_whitespace().next()?;
    if word.len() > MAX_TOKEN_BYTES {
        return None;
    }
    let word = word.to_lowercase();
    let word = ALIASES
        .iter()
        .find(|(alias, _)| *alias == word)
        .map_or(word, |(_, target)| (*target).to_string());

    let set = syntaxes();
    let mut guard = MEMO.lock().unwrap_or_else(PoisonError::into_inner);
    let memo = guard.get_or_insert_with(Memo::default);

    if memo.syntax_by_token.len() >= MAX_REMEMBERED_TOKENS {
        memo.syntax_by_token.clear();
    }
    let index = *memo.syntax_by_token.entry(word).or_insert_with_key(|word| {
        let found = set.find_syntax_by_token(word)?;
        set.syntaxes()
            .iter()
            .position(|candidate| std::ptr::eq(candidate, found))
    });
    let syntax = set.syntaxes().get(index?)?;

    if code.len() > MAX_BODY_BYTES {
        return Some(Vec::new());
    }

    let mut state = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut spans: Vec<(ByteSpan, Token)> = Vec::new();
    let mut base = 0usize;
    let mut scratch = String::new();
    let mut spent = 0u64;

    for line in code.split_inclusive('\n') {
        if line.len() > MAX_LINE_BYTES {
            break;
        }
        let cost = (line.len() as u64).pow(2);
        if spent.saturating_add(cost) > max_work {
            break;
        }
        spent += cost;
        // syntect's syntaxes expect every line to end in a newline, and the last line of a fence
        // may not; a copy with one added is parsed and the result clamped back to the real line.
        let parsed = if line.ends_with('\n') {
            line
        } else {
            scratch.clear();
            scratch.push_str(line);
            scratch.push('\n');
            scratch.as_str()
        };
        let Ok(ops) = state.parse_line(parsed, set) else {
            break;
        };

        let mut at = 0usize;
        let mut ok = true;
        for (end, op) in ops {
            let end = end.min(line.len());
            if end < at || !line.is_char_boundary(end) {
                ok = false;
                break;
            }
            if let Some(token) = classify(memo, &stack) {
                push(&mut spans, base + at, base + end, token);
            }
            at = end;
            if stack.apply(&op).is_err() {
                ok = false;
                break;
            }
        }
        if !ok {
            break;
        }
        if let Some(token) = classify(memo, &stack) {
            push(&mut spans, base + at, base + line.len(), token);
        }
        base += line.len();
    }

    Some(spans)
}

/// Appends `start..end`, merging into the previous span when they touch and agree.
fn push(spans: &mut Vec<(ByteSpan, Token)>, start: usize, end: usize, token: Token) {
    if start >= end {
        return;
    }
    if let Some((last, last_token)) = spans.last_mut()
        && *last_token == token
        && last.end().get() == start
    {
        *last = ByteSpan::new(last.start(), ByteOffset::new(end));
        return;
    }
    spans.push((
        ByteSpan::new(ByteOffset::new(start), ByteOffset::new(end)),
        token,
    ));
}

/// What `stack` is drawn as. A comment or a string anywhere in the stack governs everything inside
/// it, so a string's quotes are string-coloured rather than punctuation; otherwise the innermost
/// scope with a colour wins.
fn classify(memo: &mut Memo, stack: &ScopeStack) -> Option<Token> {
    let scopes = stack.as_slice();
    for scope in scopes {
        if let Some(token @ (Token::Comment | Token::String)) = token_for(memo, *scope) {
            return Some(token);
        }
    }
    scopes
        .iter()
        .rev()
        .find_map(|scope| token_for(memo, *scope))
}

fn token_for(memo: &mut Memo, scope: Scope) -> Option<Token> {
    *memo
        .token_by_scope
        .entry(scope)
        .or_insert_with(|| token_of(&scope.build_string()))
}

fn token_of(name: &str) -> Option<Token> {
    SCOPES
        .iter()
        .filter(|(prefix, _)| {
            name.strip_prefix(prefix)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
        })
        .max_by_key(|(prefix, _)| prefix.len())
        .map(|(_, token)| *token)
}

/// Runs `parse_line` over a few sample lines in every bundled syntax and returns the names of the
/// ones whose first error it hit. For the decision record, not for the editor.
#[doc(hidden)]
#[must_use]
pub fn syntaxes_that_error(samples: &[&str]) -> Vec<String> {
    syntaxes_that_error_in(&[], samples)
}

/// As [`syntaxes_that_error`], but only for the syntaxes named in `only`; empty means all.
#[doc(hidden)]
#[must_use]
pub fn syntaxes_that_error_in(only: &[&str], samples: &[&str]) -> Vec<String> {
    let set = syntaxes();
    let mut failing = Vec::new();
    for syntax in set.syntaxes() {
        if !only.is_empty() && !only.contains(&syntax.name.as_str()) {
            continue;
        }
        let mut state = ParseState::new(syntax);
        let failed = samples.iter().any(|sample| {
            let mut line = (*sample).to_string();
            line.push('\n');
            state.parse_line(&line, set).is_err()
        });
        if failed {
            failing.push(syntax.name.clone());
        }
    }
    failing
}

/// The name of every bundled language. For the decision record, not for the editor.
#[doc(hidden)]
#[must_use]
pub fn language_names() -> Vec<String> {
    let set = syntaxes();
    set.syntaxes()
        .iter()
        .map(|syntax| syntax.name.clone())
        .collect()
}
