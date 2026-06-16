//! pulse_pyhocon — native HOCON parser in Rust (drop-in for the `ConfigFactory.parse_string` hotspot).
//! pyhocon's bottleneck is the INTERPRETED pyparsing machinery (~1% C regex) → an ideal target.
//!
//! Covered natively: objects/arrays/scalars, dotted keys, deep merge, comments; substitutions
//!   `${path}`/`${?path}` (type preserved when alone, string concat, dotted paths, forward/backward
//!   refs, sub→sub, optional omitted, env fallback); file `include`s (tracked base dir, merged at the
//!   include point, required → FileNotFoundError); object (deep merge) and array (concat)
//!   concatenation; self-reference (`p = ${p}":/usr/bin"`).
//!
//! ISO PRINCIPLE: the native core only handles the happy path. ANY substitution-resolution failure
//!   (self-reference `a = ${a}`, self-concat `p = ${p}":x"`, path navigation through a substitution
//!   `${x.host}`, undefined variable, cycle, type mismatch…) raises `NotImplementedError` → the wrapper
//!   delegates to pyhocon (the ORACLE: it resolves, or raises the right exception). Likewise
//!   `include url(...)`/`classpath(...)`, `+=`, special quoted keys. So: never a divergence.

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use std::path::PathBuf;

#[derive(Clone, Debug)]
enum Value {
    Null,
    Bool(bool),
    Int(i64),
    BigInt(String), // integer beyond i64 → Python int (arbitrary precision)
    Float(f64),
    Str(String),
    Arr(Vec<Value>),
    Obj(Vec<(String, Value)>),
    Subst { path: Vec<String>, optional: bool },
    Concat(Vec<CSeg>),
}

/// Segment of a value concatenation (resolved to merge / concat / string depending on the types).
#[derive(Clone, Debug)]
enum CSeg {
    Val(Value),          // object / array / quoted string (unresolved)
    Text(String),        // raw unquoted text (may be whitespace only)
    Sub { path: Vec<String>, optional: bool },
}

enum RawPart {
    Quoted(String),
    Text(String),
    Sub { path: Vec<String>, optional: bool },
    Obj(Vec<(String, Value)>),
    Arr(Vec<Value>),
}

#[derive(Debug)]
enum HoconError {
    Parse(String),        // -> ValueError
    FileNotFound(String), // -> FileNotFoundError
    Unsupported(String),  // -> NotImplementedError → transparent fallback to pyhocon (wrapper)
}
impl From<&str> for HoconError {
    fn from(s: &str) -> Self {
        HoconError::Parse(s.to_string())
    }
}
impl From<String> for HoconError {
    fn from(s: String) -> Self {
        HoconError::Parse(s)
    }
}

/// Resolution-pass errors. All routed to the transparent fallback (pyhocon decides): the native core
/// only handles what it can reproduce identically.
#[derive(Debug)]
enum ResolveError {
    Subst(String),     // unresolvable substitution (non-concrete self-ref, cycle, undefined…)
    WrongType(String), // object/array inside a string concatenation
    Fallback(String),  // rendering not guaranteed iso (e.g. extreme float in concat → Python str(float))
}

struct Parser {
    c: Vec<char>,
    i: usize,
    base: PathBuf,
}

impl Parser {
    fn new(s: &str) -> Self {
        Parser { c: s.chars().collect(), i: 0, base: PathBuf::new() }
    }
    fn peek(&self) -> Option<char> {
        self.c.get(self.i).copied()
    }
    fn peek2(&self) -> Option<char> {
        self.c.get(self.i + 1).copied()
    }
    fn bump(&mut self) -> Option<char> {
        let ch = self.peek();
        if ch.is_some() {
            self.i += 1;
        }
        ch
    }

    fn skip_inline(&mut self) {
        loop {
            match self.peek() {
                Some(' ') | Some('\t') | Some('\r') => self.i += 1,
                Some('#') => self.skip_to_eol(),
                Some('/') if self.peek2() == Some('/') => self.skip_to_eol(),
                _ => break,
            }
        }
    }

    fn skip_separators(&mut self) {
        loop {
            match self.peek() {
                Some(' ') | Some('\t') | Some('\r') | Some('\n') | Some(',') => self.i += 1,
                Some('#') => self.skip_to_eol(),
                Some('/') if self.peek2() == Some('/') => self.skip_to_eol(),
                _ => break,
            }
        }
    }

    fn skip_to_eol(&mut self) {
        while let Some(ch) = self.peek() {
            self.i += 1;
            if ch == '\n' {
                break;
            }
        }
    }

    fn parse_root(&mut self) -> Result<Value, HoconError> {
        self.skip_separators();
        if self.peek() == Some('{') {
            let m = self.parse_braced_members()?;
            self.skip_separators();
            if self.peek().is_some() {
                return Err("content after the root object".into());
            }
            Ok(Value::Obj(m))
        } else {
            Ok(Value::Obj(self.parse_members_until(None)?))
        }
    }

    fn parse_braced_members(&mut self) -> Result<Vec<(String, Value)>, HoconError> {
        if self.bump() != Some('{') {
            return Err("expected '{'".into());
        }
        let members = self.parse_members_until(Some('}'))?;
        if self.bump() != Some('}') {
            return Err("expected '}'".into());
        }
        Ok(members)
    }

    fn parse_members_until(&mut self, close: Option<char>) -> Result<Vec<(String, Value)>, HoconError> {
        let mut members: Vec<(String, Value)> = Vec::new();
        loop {
            self.skip_separators();
            match self.peek() {
                None => {
                    if close.is_some() {
                        return Err("missing '}'".into());
                    }
                    break;
                }
                Some(ch) if Some(ch) == close => break,
                _ => {}
            }
            let (key, quoted) = self.parse_key()?;
            self.skip_inline();
            // `include` directive: only when NOT quoted (`"include"` is a literal key)
            if key == "include" && !quoted && !matches!(self.peek(), Some('=') | Some(':') | Some('{')) {
                self.process_include(&mut members)?;
                continue;
            }
            // `+=`: pyhocon 0.3.63 implements it in a buggy way → transparent fallback (wrapper).
            if self.peek() == Some('+') && self.peek2() == Some('=') {
                return Err(HoconError::Unsupported("'+=' operator".into()));
            }
            // Quoted key with special characters: pyhocon keeps the quotes / does not split, in a quirky
            // way → transparent fallback. A "simple identifier" quoted key is, on the other hand,
            // stripped like a bare key (directly iso behavior).
            if quoted && !is_safe_key(&key) {
                return Err(HoconError::Unsupported("quoted key with special characters".into()));
            }
            let value = match self.peek() {
                Some('=') | Some(':') => {
                    self.bump();
                    self.skip_inline();
                    self.parse_value()?
                }
                // `a { ... }`: the object IS the whole value (no greedy concatenation)
                Some('{') => Value::Obj(self.parse_braced_members()?),
                other => return Err(format!("expected '=' or ':' after the key, got {:?}", other).into()),
            };
            if quoted {
                merge_into(&mut members, key, value); // literal segment, never dot-split
            } else {
                let (head, sub) = split_head(&key, value);
                merge_into(&mut members, head, sub);
            }
        }
        Ok(members)
    }

    /// Returns (key, quoted?). A quoted key is a LITERAL segment (never dot-split).
    fn parse_key(&mut self) -> Result<(String, bool), HoconError> {
        if self.peek() == Some('"') {
            return Ok((self.parse_quoted()?, true));
        }
        let mut s = String::new();
        while let Some(ch) = self.peek() {
            if ch.is_whitespace() || matches!(ch, '=' | ':' | '{' | '}' | '[' | ']' | ',') {
                break;
            }
            if ch == '+' && self.peek2() == Some('=') {
                break; // `key+=` without space: let the += operator be detected
            }
            if ch == '#' || (ch == '/' && self.peek2() == Some('/')) {
                break;
            }
            s.push(ch);
            self.i += 1;
        }
        if s.is_empty() {
            return Err("empty key".into());
        }
        Ok((s, false))
    }

    /// A value = sequence of UNITS (object / array / quoted / substitution / text) up to the
    /// terminator. A single unit → that value; several → concatenation.
    fn parse_value(&mut self) -> Result<Value, HoconError> {
        self.skip_inline();
        let mut parts: Vec<RawPart> = Vec::new();
        loop {
            match self.peek() {
                None | Some('\n') | Some(',') | Some('}') | Some(']') => break,
                Some('#') => break,
                Some('/') if self.peek2() == Some('/') => break,
                Some('{') => parts.push(RawPart::Obj(self.parse_braced_members()?)),
                Some('[') => parts.push(RawPart::Arr(self.parse_array_items()?)),
                Some('"') => parts.push(RawPart::Quoted(self.parse_quoted()?)),
                Some('$') if self.peek2() == Some('{') => {
                    let (path, optional) = self.parse_subst()?;
                    parts.push(RawPart::Sub { path, optional });
                }
                Some(_) => {
                    let mut t = String::new();
                    loop {
                        // `#` and `//` only start a comment when preceded by whitespace (or at the
                        // start of the value) — otherwise they are literal (e.g. `a//b`, `http://x`).
                        let prev_ws = t.chars().last().is_none_or(|c| c.is_whitespace());
                        match self.peek() {
                            None | Some('\n') | Some(',') | Some('}') | Some(']') => break,
                            Some('#') if prev_ws => break,
                            Some('/') if self.peek2() == Some('/') && prev_ws => break,
                            Some('"') | Some('{') | Some('[') => break,
                            Some('$') if self.peek2() == Some('{') => break,
                            Some(ch) => {
                                t.push(ch);
                                self.i += 1;
                            }
                        }
                    }
                    parts.push(RawPart::Text(t));
                }
            }
        }
        if parts.is_empty() {
            // empty value (`a =`): pyhocon's rule is inconsistent (str "" or ParseException depending
            // on context) → transparent fallback to pyhocon.
            return Err(HoconError::Unsupported("empty value".into()));
        }
        build_value(parts)
    }

    fn parse_subst(&mut self) -> Result<(Vec<String>, bool), HoconError> {
        self.bump();
        self.bump(); // ${
        let optional = if self.peek() == Some('?') {
            self.bump();
            true
        } else {
            false
        };
        let mut p = String::new();
        loop {
            match self.bump() {
                None => return Err("missing '}' in substitution".into()),
                Some('}') => break,
                Some(ch) => p.push(ch),
            }
        }
        let path = p.trim().split('.').map(|s| s.trim().to_string()).collect();
        Ok((path, optional))
    }

    fn parse_array_items(&mut self) -> Result<Vec<Value>, HoconError> {
        self.bump(); // '['
        let mut items = Vec::new();
        loop {
            self.skip_separators();
            match self.peek() {
                Some(']') => {
                    self.bump();
                    break;
                }
                None => return Err("missing ']'".into()),
                _ => {}
            }
            items.push(self.parse_value()?);
        }
        Ok(items)
    }

    fn parse_quoted(&mut self) -> Result<String, HoconError> {
        self.bump(); // '"'
        let mut s = String::new();
        loop {
            match self.bump() {
                None => return Err("missing closing quote".into()),
                Some('"') => break,
                Some('\\') => match self.bump() {
                    Some('n') => s.push('\n'),
                    Some('t') => s.push('\t'),
                    Some('r') => s.push('\r'),
                    Some('"') => s.push('"'),
                    Some('\\') => s.push('\\'),
                    Some('/') => s.push('/'),
                    Some(other) => {
                        s.push('\\');
                        s.push(other);
                    }
                    None => return Err("incomplete escape".into()),
                },
                Some(ch) => s.push(ch),
            }
        }
        Ok(s)
    }

    fn process_include(&mut self, members: &mut Vec<(String, Value)>) -> Result<(), HoconError> {
        let mut raw = String::new();
        loop {
            match self.peek() {
                None | Some('\n') | Some(',') | Some('}') | Some(']') => break,
                Some('#') => break,
                Some('/') if self.peek2() == Some('/') => break,
                Some(ch) => {
                    raw.push(ch);
                    self.i += 1;
                }
            }
        }
        let spec = parse_include_spec(raw.trim())?;
        if !spec.supported {
            // include url(...)/classpath(...): pyhocon fetches the resource (and merges it, or raises).
            // We must NOT silently ignore it (divergence) → transparent fallback.
            return Err(HoconError::Unsupported(format!("include {}(...) out of scope", spec.kind)));
        }
        let full = if self.base.as_os_str().is_empty() {
            PathBuf::from(&spec.path)
        } else {
            self.base.join(&spec.path)
        };
        match std::fs::read_to_string(&full) {
            Ok(content) => {
                let mut sub = Parser::new(&content);
                sub.base = full.parent().map(|p| p.to_path_buf()).unwrap_or_default();
                if let Value::Obj(sub_members) = sub.parse_root()? {
                    for (k, v) in sub_members {
                        merge_into(members, k, v);
                    }
                }
                Ok(())
            }
            Err(_) => {
                if spec.required {
                    Err(HoconError::FileNotFound(spec.path))
                } else {
                    Ok(())
                }
            }
        }
    }
}

struct IncludeSpec {
    path: String,
    required: bool,
    kind: &'static str,
    supported: bool,
}

fn strip_call<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let prefix = format!("{}(", name);
    if s.starts_with(&prefix) && s.ends_with(')') {
        Some(&s[prefix.len()..s.len() - 1])
    } else {
        None
    }
}

fn parse_include_spec(raw: &str) -> Result<IncludeSpec, HoconError> {
    let mut s = raw.trim();
    let mut required = false;
    if let Some(inner) = strip_call(s, "required") {
        required = true;
        s = inner.trim();
    }
    let mut kind = "file";
    let mut supported = true;
    for k in ["url", "classpath", "file"] {
        if let Some(inner) = strip_call(s, k) {
            kind = k;
            supported = k == "file";
            s = inner.trim();
            break;
        }
    }
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        Ok(IncludeSpec { path: s[1..s.len() - 1].to_string(), required, kind, supported })
    } else {
        // malformed include (unquoted path…): pyhocon has its own semantics/error → fallback.
        Err(HoconError::Unsupported(format!("unquoted include: {:?}", raw)))
    }
}

/// Typed HOCON keyword (true/false/null, case-insensitive). pyhocon TYPES them even in the middle of an
/// unquoted run (and `null` → `NoneValue` repr WITH a memory address, hence non-deterministic/unreplicable).
fn is_bare_kw(tok: &str) -> bool {
    tok.eq_ignore_ascii_case("true") || tok.eq_ignore_ascii_case("false") || tok.eq_ignore_ascii_case("null")
}

fn build_value(parts: Vec<RawPart>) -> Result<Value, HoconError> {
    let non_ws: Vec<usize> = parts
        .iter()
        .enumerate()
        .filter(|(_, p)| !matches!(p, RawPart::Text(t) if t.trim().is_empty()))
        .map(|(i, _)| i)
        .collect();

    if non_ws.len() == 1 {
        return Ok(match &parts[non_ws[0]] {
            RawPart::Sub { path, optional } => Value::Subst { path: path.clone(), optional: *optional },
            RawPart::Quoted(s) => Value::Str(s.clone()),
            RawPart::Obj(m) => Value::Obj(m.clone()),
            RawPart::Arr(a) => Value::Arr(a.clone()),
            RawPart::Text(t) => {
                let tr = t.trim();
                // unquoted MULTI-token value containing a bare keyword → pyhocon types it (divergence,
                // and `null` is unreplicable) → fallback. (A lone keyword is correctly typed by classify.)
                if tr.split_whitespace().count() > 1 && tr.split_whitespace().any(is_bare_kw) {
                    return Err(HoconError::Unsupported("bare keyword (true/false/null) in multi-token value".into()));
                }
                classify(tr)
            }
        });
    }
    // several units → concatenation. A text segment containing a bare keyword would be typed by
    // pyhocon (≠ text on the native side) → transparent fallback.
    for p in &parts {
        if let RawPart::Text(t) = p {
            if t.split_whitespace().any(is_bare_kw) {
                return Err(HoconError::Unsupported("bare keyword (true/false/null) in concatenation".into()));
            }
        }
    }
    let segs = parts
        .into_iter()
        .map(|p| match p {
            RawPart::Quoted(q) => CSeg::Val(Value::Str(q)),
            RawPart::Obj(m) => CSeg::Val(Value::Obj(m)),
            RawPart::Arr(a) => CSeg::Val(Value::Arr(a)),
            RawPart::Sub { path, optional } => CSeg::Sub { path, optional },
            RawPart::Text(t) => CSeg::Text(t),
        })
        .collect();
    Ok(Value::Concat(segs))
}

fn classify(t: &str) -> Value {
    if t.eq_ignore_ascii_case("true") {
        return Value::Bool(true);
    }
    if t.eq_ignore_ascii_case("false") {
        return Value::Bool(false);
    }
    if t.eq_ignore_ascii_case("null") {
        return Value::Null;
    }
    if let Ok(i) = t.parse::<i64>() {
        return Value::Int(i);
    }
    if is_int_token(t) {
        return Value::BigInt(t.to_string()); // valid integer but beyond i64 → Python int
    }
    if is_hocon_float(t) {
        if let Ok(f) = t.parse::<f64>() {
            return Value::Float(f);
        }
    }
    Value::Str(t.to_string())
}

/// "Simple identifier" quoted key (stripped like a bare key, iso). Otherwise → pyhocon fallback.
fn is_safe_key(k: &str) -> bool {
    !k.is_empty() && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Token = valid literal integer (`[+-]?\d+`), even beyond i64.
fn is_int_token(t: &str) -> bool {
    let b = t.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    if i >= b.len() {
        return false;
    }
    b[i..].iter().all(|c| c.is_ascii_digit())
}

fn is_hocon_float(t: &str) -> bool {
    let b = t.as_bytes();
    let n = b.len();
    let mut i = 0;
    if i < n && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let mut int_d = 0;
    while i < n && b[i].is_ascii_digit() {
        i += 1;
        int_d += 1;
    }
    let (mut has_dot, mut frac_d) = (false, 0);
    if i < n && b[i] == b'.' {
        has_dot = true;
        i += 1;
        while i < n && b[i].is_ascii_digit() {
            i += 1;
            frac_d += 1;
        }
    }
    let mut has_exp = false;
    if i < n && (b[i] == b'e' || b[i] == b'E') {
        has_exp = true;
        i += 1;
        if i < n && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        let mut exp_d = 0;
        while i < n && b[i].is_ascii_digit() {
            i += 1;
            exp_d += 1;
        }
        if exp_d == 0 {
            return false;
        }
    }
    i == n && (int_d + frac_d) >= 1 && ((has_dot && frac_d >= 1) || has_exp)
}

fn split_head(key: &str, value: Value) -> (String, Value) {
    match key.find('.') {
        None => (key.to_string(), value),
        Some(pos) => {
            let head = key[..pos].to_string();
            let (k2, v2) = split_head(&key[pos + 1..], value);
            (head, Value::Obj(vec![(k2, v2)]))
        }
    }
}

/// Fully RESOLVED value (no pending substitution/concat, recursively). Native self-reference is only
/// safe toward such a value: if the previous value contains an unresolved `${…}`, pyhocon's semantics
/// differ (it may raise) → we then leave the case to the fallback.
fn is_concrete(v: &Value) -> bool {
    match v {
        Value::Subst { .. } | Value::Concat(_) => false,
        Value::Arr(items) => items.iter().all(is_concrete),
        Value::Obj(m) => m.iter().all(|(_, vv)| is_concrete(vv)),
        _ => true, // Null / Bool / Int / BigInt / Float / Str
    }
}

/// Navigate the previous value `prior` along the rest of a substitution path (`${k.rest…}`).
/// Empty `rest` → `prior` itself. None if navigation fails OR if the reached value is not concrete
/// (→ we do not rewrite, iso fallback).
fn navigate_prior(prior: &Value, rest: &[String]) -> Option<Value> {
    let mut cur = prior;
    for seg in rest {
        match cur {
            Value::Obj(m) => cur = &m.iter().find(|(k, _)| k == seg)?.1,
            _ => return None,
        }
    }
    if is_concrete(cur) {
        Some(cur.clone())
    } else {
        None
    }
}

/// HOCON SELF-REFERENCE resolution: when the incoming value (overriding key `key`) contains
/// `${key}` / `${key.sub}` at the top level OR in a `Concat` segment, that `${…}` resolves to the
/// **previous** value of `key` (idiom `p = ${p}":/usr/bin"`, `a = ${a} [2]`, `a = ${a} {c=2}`).
/// We only rewrite these immediate self-refs; absolute-path nested self-refs or navigation through a
/// substitution stay un-rewritten → resolution failure → fallback (iso).
fn substitute_self_ref(value: Value, key: &str, prior: &Value) -> Value {
    let is_self = |path: &[String]| path.first().map(|s| s == key).unwrap_or(false);
    match value {
        Value::Subst { path, optional } => {
            if is_self(&path) {
                if let Some(v) = navigate_prior(prior, &path[1..]) {
                    return v;
                }
            }
            Value::Subst { path, optional }
        }
        Value::Concat(segs) => {
            let new_segs = segs
                .into_iter()
                .map(|seg| match seg {
                    CSeg::Sub { path, optional } => {
                        if is_self(&path) {
                            if let Some(v) = navigate_prior(prior, &path[1..]) {
                                return CSeg::Val(v);
                            }
                        }
                        CSeg::Sub { path, optional }
                    }
                    other => other,
                })
                .collect();
            Value::Concat(new_segs)
        }
        other => other,
    }
}

fn merge_into(members: &mut Vec<(String, Value)>, key: String, value: Value) {
    if let Some(idx) = members.iter().position(|(k, _)| *k == key) {
        // self-reference: `${key}` in the value overriding `key` → PREVIOUS value (HOCON).
        let value = substitute_self_ref(value, &key, &members[idx].1);
        match (&mut members[idx].1, value) {
            (Value::Obj(existing), Value::Obj(incoming)) => {
                for (k, v) in incoming {
                    merge_into(existing, k, v);
                }
            }
            (s, v) => *s = v,
        }
    } else {
        members.push((key, value));
    }
}

// ---------- Resolution pass ----------

fn get_raw<'a>(root: &'a Value, path: &[String]) -> Option<&'a Value> {
    let mut cur = root;
    for seg in path {
        match cur {
            Value::Obj(m) => cur = &m.iter().find(|(k, _)| k == seg)?.1,
            _ => return None,
        }
    }
    Some(cur)
}

fn resolve_node(node: &Value, root: &Value, stack: &mut Vec<String>) -> Result<Option<Value>, ResolveError> {
    match node {
        Value::Null | Value::Bool(_) | Value::Int(_) | Value::BigInt(_) | Value::Float(_)
        | Value::Str(_) => Ok(Some(node.clone())),
        Value::Arr(items) => {
            let mut out = Vec::new();
            for it in items {
                if let Some(v) = resolve_node(it, root, stack)? {
                    out.push(v);
                }
            }
            Ok(Some(Value::Arr(out)))
        }
        Value::Obj(members) => {
            let mut out = Vec::new();
            for (k, v) in members {
                match resolve_node(v, root, stack)? {
                    // pyhocon quirk: a substitution (as the whole value) resolving to null drops the key
                    Some(Value::Null) if matches!(v, Value::Subst { .. }) => {}
                    Some(rv) => out.push((k.clone(), rv)),
                    None => {}
                }
            }
            Ok(Some(Value::Obj(out)))
        }
        Value::Subst { path, optional } => resolve_subst(path, *optional, root, stack),
        Value::Concat(segs) => resolve_concat(segs, root, stack),
    }
}

fn resolve_subst(
    path: &[String],
    optional: bool,
    root: &Value,
    stack: &mut Vec<String>,
) -> Result<Option<Value>, ResolveError> {
    let key = path.join(".");
    if stack.contains(&key) {
        return Err(ResolveError::Subst(format!("circular substitution ${{{}}}", key)));
    }
    if let Some(raw) = get_raw(root, path) {
        stack.push(key);
        let r = resolve_node(raw, root, stack);
        stack.pop();
        return r;
    }
    if let Ok(v) = std::env::var(&key) {
        return Ok(Some(Value::Str(v)));
    }
    if optional {
        Ok(None)
    } else {
        Err(ResolveError::Subst(format!("Cannot resolve variable ${{{}}}", key)))
    }
}

/// Resolve a concatenation: deep merge if all units are objects, concat if all are arrays, otherwise
/// string concatenation (an object/array inside a string → ConfigWrongTypeException).
fn resolve_concat(segs: &[CSeg], root: &Value, stack: &mut Vec<String>) -> Result<Option<Value>, ResolveError> {
    struct Unit {
        val: Option<Value>, // None for text, or an absent optional substitution
        text: Option<String>,
        ws: bool,
    }
    let mut units = Vec::new();
    for seg in segs {
        match seg {
            CSeg::Text(t) => units.push(Unit { val: None, text: Some(t.clone()), ws: t.trim().is_empty() }),
            CSeg::Val(v) => units.push(Unit { val: resolve_node(v, root, stack)?, text: None, ws: false }),
            CSeg::Sub { path, optional } => {
                units.push(Unit { val: resolve_subst(path, *optional, root, stack)?, text: None, ws: false })
            }
        }
    }
    // meaningful units (excluding purely-whitespace text)
    let meaningful: Vec<&Unit> = units.iter().filter(|u| !(u.text.is_some() && u.ws)).collect();
    let has_text = meaningful.iter().any(|u| u.text.is_some());
    let present: Vec<&Value> = meaningful.iter().filter_map(|u| u.val.as_ref()).collect();

    if !has_text && !present.is_empty() && present.iter().all(|v| matches!(v, Value::Obj(_))) {
        let mut acc: Vec<(String, Value)> = Vec::new();
        for v in present {
            if let Value::Obj(m) = v {
                for (k, val) in m {
                    merge_into(&mut acc, k.clone(), val.clone());
                }
            }
        }
        return Ok(Some(Value::Obj(acc)));
    }
    if !has_text && !present.is_empty() && present.iter().all(|v| matches!(v, Value::Arr(_))) {
        let mut acc = Vec::new();
        for v in present {
            if let Value::Arr(a) = v {
                acc.extend(a.clone());
            }
        }
        return Ok(Some(Value::Arr(acc)));
    }
    // no present value and no meaningful text (e.g. ${?x}${?y} all absent) → key omitted
    if present.is_empty() && !has_text {
        return Ok(None);
    }
    // otherwise: string concatenation
    let mut out = String::new();
    for u in &units {
        if let Some(t) = &u.text {
            out.push_str(t);
        } else if let Some(v) = &u.val {
            out.push_str(&render_scalar(v)?);
        }
    }
    Ok(Some(Value::Str(out.trim().to_string())))
}

fn render_scalar(v: &Value) -> Result<String, ResolveError> {
    Ok(match v {
        Value::Null => String::new(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Int(i) => i.to_string(),
        Value::BigInt(s) => s.clone(),
        // Rust formats floats WITHOUT scientific notation; Python `str(float)` uses it outside
        // [1e-4, 1e16). For such extreme (or non-finite) floats in a string concatenation the rendering
        // would diverge → transparent fallback (pyhocon renders). "Normal" floats stay native.
        Value::Float(f) if f.is_finite() && (*f == 0.0 || (f.abs() >= 1e-4 && f.abs() < 1e16)) => render_float(*f),
        Value::Float(_) => {
            return Err(ResolveError::Fallback(
                "float outside the iso-renderable range in string concatenation".into(),
            ))
        }
        Value::Str(s) => s.clone(),
        Value::Obj(_) | Value::Arr(_) => {
            return Err(ResolveError::WrongType(
                "object/array not allowed in a string concatenation".into(),
            ))
        }
        Value::Subst { .. } | Value::Concat(_) => unreachable!("unresolved"),
    })
}

fn render_float(f: f64) -> String {
    let s = format!("{}", f);
    if s.contains('.') || s.contains('e') || s.contains('E') || s.contains("inf") || s.contains("NaN") {
        s
    } else {
        format!("{}.0", s)
    }
}

fn value_to_py<'py>(py: Python<'py>, v: &Value) -> PyResult<Bound<'py, PyAny>> {
    Ok(match v {
        Value::Null => py.None().into_bound(py),
        Value::Bool(b) => (*b).into_pyobject(py)?.to_owned().into_any(),
        Value::Int(i) => (*i).into_pyobject(py)?.into_any(),
        Value::BigInt(s) => py
            .import("builtins")?
            .getattr("int")?
            .call1((s.as_str(),))?
            .into_any(),
        Value::Float(f) => (*f).into_pyobject(py)?.into_any(),
        Value::Str(s) => s.into_pyobject(py)?.into_any(),
        Value::Arr(items) => {
            let l = PyList::empty(py);
            for it in items {
                l.append(value_to_py(py, it)?)?;
            }
            l.into_any()
        }
        Value::Obj(pairs) => {
            let d = PyDict::new(py);
            for (k, val) in pairs {
                d.set_item(k, value_to_py(py, val)?)?;
            }
            d.into_any()
        }
        Value::Subst { .. } | Value::Concat(_) => {
            return Err(pyo3::exceptions::PyRuntimeError::new_err("unresolved node"))
        }
    })
}

/// Parse HOCON → nested dict (the Python wrapper then wraps it into a `ConfigTree`).
/// `base`: directory used to resolve `include`s (None = cwd, like `parse_string`; `parse_file` passes
/// the file's directory, like pyhocon).
#[pyfunction]
#[pyo3(signature = (s, base=None))]
fn parse(py: Python<'_>, s: &str, base: Option<&str>) -> PyResult<PyObject> {
    let mut parser = Parser::new(s);
    if let Some(b) = base {
        parser.base = PathBuf::from(b);
    }
    let tree = match parser.parse_root() {
        Ok(t) => t,
        Err(HoconError::FileNotFound(p)) => {
            return Err(pyo3::exceptions::PyFileNotFoundError::new_err(format!(
                "Cannot include required file: {}",
                p
            )))
        }
        Err(HoconError::Parse(m)) => return Err(pyo3::exceptions::PyValueError::new_err(m)),
        // Transparent-fallback signal: the Python wrapper catches it and delegates to pyhocon.
        Err(HoconError::Unsupported(m)) => return Err(pyo3::exceptions::PyNotImplementedError::new_err(m)),
    };
    let mut stack = Vec::new();
    let resolved = match resolve_node(&tree, &tree, &mut stack) {
        Ok(r) => r.unwrap_or(Value::Obj(Vec::new())),
        // ANY resolution failure → transparent fallback to pyhocon (the oracle decides). The native
        // core only handles the happy path; for anything it cannot resolve — self-reference
        // (`a = ${a}`), self-concat (`p = ${p}":x"`), path navigation through a substitution
        // (`${x.host}` where x=${base})… which pyhocon RESOLVES, as well as genuine errors (undefined
        // variable, cycle, type mismatch) which pyhocon RAISES — we delegate. Iso guaranteed.
        Err(ResolveError::Subst(m)) | Err(ResolveError::WrongType(m)) | Err(ResolveError::Fallback(m)) => {
            return Err(pyo3::exceptions::PyNotImplementedError::new_err(m))
        }
    };
    Ok(value_to_py(py, &resolved)?.unbind())
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(parse, m)?)?;
    m.add("__backend__", "rust")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn resolved(s: &str) -> Value {
        let t = Parser::new(s).parse_root().unwrap();
        let mut st = Vec::new();
        resolve_node(&t, &t, &mut st).unwrap().unwrap()
    }
    fn get<'a>(v: &'a Value, k: &str) -> &'a Value {
        match v {
            Value::Obj(m) => &m.iter().find(|(kk, _)| kk == k).unwrap().1,
            _ => panic!(),
        }
    }
    #[test]
    fn obj_concat_merge() {
        let v = resolved("o1={x=1}\no2={y=2}\nm=${o1}${o2}");
        assert!(matches!(get(&v, "m"), Value::Obj(m) if m.len() == 2));
    }
    #[test]
    fn obj_literal_merge_override() {
        let v = resolved("m = {x=1} {x=9, y=2}");
        if let Value::Obj(m) = get(&v, "m") {
            assert!(matches!(m.iter().find(|(k, _)| k == "x").unwrap().1, Value::Int(9)));
            assert_eq!(m.len(), 2);
        } else {
            panic!()
        }
    }
    #[test]
    fn array_concat() {
        let v = resolved("a1=[1,2]\na2=[3,4]\nc=${a1}${a2}");
        assert!(matches!(get(&v, "c"), Value::Arr(a) if a.len() == 4));
    }
    #[test]
    fn array_literal_concat() {
        assert!(matches!(get(&resolved("c = [1] [2] [3]"), "c"), Value::Arr(a) if a.len() == 3));
    }
    #[test]
    fn mixed_obj_scalar_wrongtype() {
        let t = Parser::new("o={x=1}\nm=${o} foo").parse_root().unwrap();
        let mut st = Vec::new();
        assert!(matches!(resolve_node(&t, &t, &mut st), Err(ResolveError::WrongType(_))));
    }
    #[test]
    fn subst_type_preserved_and_string_concat() {
        assert!(matches!(get(&resolved("a=5\nb=${a}"), "b"), Value::Int(5)));
        assert!(matches!(get(&resolved("h=host\nu=\"http://\"${h}"), "u"), Value::Str(s) if s == "http://host"));
    }
}
