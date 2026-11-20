//! Escaping for every text format FFmpeg parses on Cut's behalf.
//!
//! One function per parsing level, so a value that did not come from Cut (a
//! path inside a received project, a folder name, caption text) can never add
//! syntax of its own. The rules follow FFmpeg's documentation ("Quoting and
//! escaping" in ffmpeg-utils, "Notes on filtergraph escaping" in
//! ffmpeg-filters, the concat demuxer syntax in ffmpeg-formats) and its parser,
//! `av_get_token`:
//!
//! * a backslash makes the next character literal;
//! * everything between single quotes is literal, including a backslash; a
//!   quote itself cannot be quoted, so it is written as close-quote, `\'`,
//!   reopen-quote;
//! * unescaped, unquoted whitespace at either end of a token is dropped.
//!
//! A filtergraph is parsed twice with those rules: first the graph level
//! splits filters at `[ ] , ;`, then each filter's option level splits options
//! at `:`. A concat list is one token per line, and a line ends at CR or LF.
//! An ASS subtitle file is not FFmpeg syntax; libass reads it, and its own
//! rules are documented at [`ass_text`].

use cut_core::{error_codes, CutError};
use std::path::Path;

/// Characters `av_get_token` treats as surrounding whitespace.
const TOKEN_WHITESPACE: [char; 4] = [' ', '\n', '\t', '\r'];

/// Option level of a filtergraph: makes `value` one literal option value.
///
/// Escapes `\`, `'` and the option separator `:` with a backslash, and the
/// whitespace at either end (which the parser would otherwise drop). The
/// result is still subject to the graph level: embed it with
/// [`filter_graph_quote`].
pub fn filter_option_value(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let first_kept = chars
        .iter()
        .position(|c| !TOKEN_WHITESPACE.contains(c))
        .unwrap_or(chars.len());
    let last_kept = chars
        .iter()
        .rposition(|c| !TOKEN_WHITESPACE.contains(c))
        .map_or(0, |i| i + 1);
    let mut out = String::with_capacity(value.len() + 8);
    for (i, c) in chars.iter().enumerate() {
        let edge_space = i < first_kept || i >= last_kept;
        if matches!(c, '\\' | '\'' | ':') || edge_space {
            out.push('\\');
        }
        out.push(*c);
    }
    out
}

/// Graph level of a filtergraph: makes `text` (already escaped for the option
/// level) one literal piece of a filter description, so `[ ] , ;` and line
/// breaks inside it cannot start a new filter, link label or chain.
pub fn filter_graph_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// How a path is written into a filter option. FFmpeg's filters that read a
/// file (`lut3d`, `ass`, `sendcmd`, `vidstab*`) open the text as a plain local
/// file name; no protocol is involved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathStyle {
    /// `\` separates folders and never occurs inside a name, so it is written
    /// as `/` (Windows accepts both); this keeps drive and UNC paths readable.
    Windows,
    /// `\` is an ordinary character of a file name and is kept as itself.
    Unix,
}

impl PathStyle {
    /// The style of the system this build runs on.
    pub const fn native() -> Self {
        if cfg!(windows) {
            PathStyle::Windows
        } else {
            PathStyle::Unix
        }
    }
}

/// A path as one literal filter option value inside a filtergraph, for the
/// given path style. See [`filter_path`].
pub fn filter_path_for(path_text: &str, style: PathStyle) -> String {
    let text = strip_verbatim_prefix(path_text);
    let text = match style {
        PathStyle::Windows => text.replace('\\', "/"),
        PathStyle::Unix => text.to_string(),
    };
    filter_graph_quote(&filter_option_value(&text))
}

/// A path as one literal filter option value inside a filtergraph
/// (`lut3d=file=<here>`, `ass=filename=<here>`). Callers embed the result
/// without adding quotes of their own.
pub fn filter_path(path: &Path) -> String {
    filter_path_for(&path.display().to_string(), PathStyle::native())
}

/// Windows extended-length prefixes (`\\?\C:\…`, `\\?\UNC\srv\…`) are not
/// understood by FFmpeg's own path handling; the plain form names the same
/// file.
pub(crate) fn strip_verbatim_prefix(text: &str) -> std::borrow::Cow<'_, str> {
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}").into();
    }
    text.strip_prefix(r"\\?\").unwrap_or(text).into()
}

/// One `file` line of a concat list for `path`.
///
/// The concat demuxer reads one directive per line and the path as one
/// `av_get_token` token, so the path is single-quoted (backslashes are then
/// literal) and each quote is written as `'\''`. A line break cannot be
/// written in this format at all; a path containing one is refused, because
/// the rest of it would be read as further directives of the list.
pub fn concat_file_line(path: &Path) -> Result<String, CutError> {
    let text = path.display().to_string();
    let text = strip_verbatim_prefix(&text);
    if text.contains(['\n', '\r']) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "a folder or file name contains a line break",
            format!("FFmpeg cannot be given {text:?} in a list of files, so Cut does not use it"),
        )
        .with_suggested_action(
            "rename the project folder (or the file) so its name has no line break, then try again",
        ));
    }
    Ok(format!("file '{}'", text.replace('\'', r"'\''")))
}

/// Caption text as the Text field of an ASS `Dialogue` line, shown exactly as
/// written.
///
/// libass reads the file line by line (a line ends at LF or CR), treats
/// `{...}` as override tags (position, colour, drawing, font) and `\N`, `\n`,
/// `\h` as a line break or hard space; `\{` and `\}` show a literal brace.
/// So every line break becomes `\N` (the visible break the person typed),
/// braces are escaped, and a backslash is followed by U+2060 WORD JOINER (an
/// invisible character) so it cannot pair with the next character into one of
/// those sequences. Ordinary text without `\`, `{`, `}` or CR is unchanged.
pub fn ass_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push_str(r"\N");
            }
            '\n' => out.push_str(r"\N"),
            '\\' => out.push_str("\\\u{2060}"),
            '{' => out.push_str(r"\{"),
            '}' => out.push_str(r"\}"),
            _ => out.push(c),
        }
    }
    out
}

/// Checks a value that Cut writes into a comma-separated ASS field it does
/// not escape (a style name, a font name). Such a field ends at a comma and
/// its line ends at a line break, so either would shift or add fields.
pub fn ass_field(kind: &str, value: &str) -> Result<(), CutError> {
    if value.contains([',', '\n', '\r']) || value.chars().any(char::is_control) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("caption {kind} {value:?} cannot be used in a subtitle file"),
            format!("a caption {kind} must not contain a comma, a line break or a control character"),
        )
        .with_suggested_action(format!(
            "rename the caption {kind} with captions.set_style (letters, digits, spaces and dashes work), then render again"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
