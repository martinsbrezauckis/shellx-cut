//! String-level checks of each escaping level against a line-by-line port of
//! FFmpeg's tokenizer. The real-tool checks are in `real_tool_tests.rs`.

use super::*;

/// Port of FFmpeg 8.0.1 `av_get_token` (libavutil/avstring.c): returns the
/// token and the unparsed rest (starting at the terminator, if any).
fn av_get_token<'a>(buf: &'a str, term: &str) -> (String, &'a str) {
    let ws = [' ', '\n', '\t', '\r'];
    let bytes: Vec<char> = buf.chars().collect();
    let mut p = 0;
    while p < bytes.len() && ws.contains(&bytes[p]) {
        p += 1;
    }
    let mut out: Vec<char> = Vec::new();
    let mut end = 0;
    while p < bytes.len() && !term.contains(bytes[p]) {
        let c = bytes[p];
        p += 1;
        if c == '\\' && p < bytes.len() {
            out.push(bytes[p]);
            p += 1;
            end = out.len();
        } else if c == '\'' {
            while p < bytes.len() && bytes[p] != '\'' {
                out.push(bytes[p]);
                p += 1;
            }
            if p < bytes.len() {
                p += 1;
                end = out.len();
            }
        } else {
            out.push(c);
        }
    }
    while out.len() > end && out.last().is_some_and(|c| ws.contains(c)) {
        out.pop();
    }
    let rest_start: usize = bytes[..p].iter().map(|c| c.len_utf8()).sum();
    (out.into_iter().collect(), &buf[rest_start..])
}

/// What FFmpeg makes of `<filter>=<key>=<embedded>` followed by more graph
/// text: the graph level token (`[],;` end it), then the option level value
/// of `key` (`:` ends it). Returns (value, what follows the value inside the
/// filter's options, what follows the filter in the graph).
fn parse_one_option(embedded: &str, graph_tail: &str) -> (String, String, String) {
    let desc = format!("lut3d=file={embedded}{graph_tail}");
    let (name, rest) = av_get_token(&desc, "=,;[");
    assert_eq!(name, "lut3d");
    let rest = rest.strip_prefix('=').unwrap();
    let (opts, after_filter) = av_get_token(rest, "[],;");
    let opts_rest = opts.strip_prefix("file=").unwrap();
    let (value, after_value) = av_get_token(opts_rest, ":");
    (value, after_value.to_string(), after_filter.to_string())
}

const HOSTILE: &[&str] = &[
    "/p/x.cube",
    "/p/it's.cube",
    r"/p/back\slash.cube",
    r"/p/a\'b.cube",
    "/p/x.lut:interp=nearest,movie=secret.mp4",
    "/p/a;b[c]d,e.cube",
    "/p/100%.cube",
    "/p/a=b.cube",
    "/p/new\nline.cube",
    " /p/leading space.cube",
    "/p/trailing space.cube ",
    "/p/tab\t.cube\t",
    "-starts-with-dash.cube",
    r"C:\Users\Example\proj\burnin.ass",
    r"\\server\share\look.cube",
    r"\\?\C:\Users\Example\proj\look.cube",
    r"\\?\UNC\server\share\look.cube",
    "/p/'];movie=/etc/passwd[x];[y'",
    r"/p/\':lut3d=file=/etc/x\\",
];

#[test]
fn filter_path_round_trips_through_both_levels_on_unix() {
    for raw in HOSTILE {
        let expected = strip_verbatim_prefix(raw).into_owned();
        let embedded = filter_path_for(raw, PathStyle::Unix);
        let (value, after_value, after_filter) = parse_one_option(&embedded, ",scale=2:2[out]");
        assert_eq!(value, expected, "{raw:?} -> {embedded}");
        assert_eq!(after_value, "", "{raw:?} leaked into the options");
        assert_eq!(
            after_filter, ",scale=2:2[out]",
            "{raw:?} leaked into the graph"
        );
    }
}

#[test]
fn filter_path_round_trips_through_both_levels_on_windows() {
    for raw in HOSTILE {
        let expected = strip_verbatim_prefix(raw).replace('\\', "/");
        let embedded = filter_path_for(raw, PathStyle::Windows);
        let (value, after_value, after_filter) = parse_one_option(&embedded, ";[a]null[b]");
        assert_eq!(value, expected, "{raw:?} -> {embedded}");
        assert_eq!(after_value, "");
        assert_eq!(after_filter, ";[a]null[b]");
    }
}

#[test]
fn filter_path_keeps_the_verified_form_of_ordinary_paths() {
    // Byte-identical to the form shipped before this module for every path
    // without a backslash on Unix, an apostrophe or edge whitespace.
    assert_eq!(
        filter_path_for(r"C:\Users\Example\proj\burnin.ass", PathStyle::Windows),
        r"'C\:/Users/Example/proj/burnin.ass'"
    );
    assert_eq!(
        filter_path_for("/home/u/My Clips/burnin.ass", PathStyle::Unix),
        "'/home/u/My Clips/burnin.ass'"
    );
    assert_eq!(
        filter_path_for(r"\\?\UNC\srv\share\x.cube", PathStyle::Windows),
        "'//srv/share/x.cube'"
    );
}

#[test]
fn unix_backslash_names_the_same_file() {
    // Before: `\` was rewritten to `/`, so `a\..\..\x` reached FFmpeg as
    // `a/../../x`, a different file.
    let embedded = filter_path_for(r"/p/a\..\..\x.cube", PathStyle::Unix);
    let (value, _, _) = parse_one_option(&embedded, "");
    assert_eq!(value, r"/p/a\..\..\x.cube");
}

#[test]
fn filter_option_value_escapes_separators_and_edges() {
    assert_eq!(filter_option_value("a:b"), r"a\:b");
    assert_eq!(filter_option_value(r"a\b"), r"a\\b");
    assert_eq!(filter_option_value("it's"), r"it\'s");
    assert_eq!(filter_option_value(" a b "), r"\ a b\ ");
    assert_eq!(filter_option_value(""), "");
}

#[test]
fn concat_line_round_trips_and_cannot_add_directives() {
    for raw in [
        "/p/seg.mp4",
        "/p/Bob's project/seg.mp4",
        r"/p/back\slash/seg.mp4",
        "/p/two  spaces/seg.mp4",
        "/p/#hash/seg.mp4",
        r"C:\Users\O'Brien\proj\seg.mp4",
        r"\\?\C:\Users\Example\project\seg_01.mp4",
    ] {
        let line = concat_file_line(Path::new(raw)).unwrap();
        assert!(!line.contains(['\n', '\r']));
        let rest = line.strip_prefix("file ").unwrap();
        let (token, tail) = av_get_token(rest, " \t\r\n");
        assert_eq!(token, strip_verbatim_prefix(raw), "{line}");
        assert_eq!(tail, "", "{line}");
    }
}

#[test]
fn concat_line_refuses_line_breaks_with_a_reason() {
    for raw in [
        "/p/x'\noption format_whitelist tty\nfile '/home/u/notes.txt",
        "/p/x\rfile '/etc/hosts'",
    ] {
        let err = concat_file_line(Path::new(raw)).unwrap_err();
        assert_eq!(err.code, error_codes::INVALID_ARGS);
        assert!(err.message.contains("line break"), "{}", err.message);
        assert!(
            err.suggested_action
                .as_deref()
                .is_some_and(|a| a.contains("rename")),
            "{err:?}"
        );
    }
}

#[test]
fn ass_text_shows_tags_and_line_breaks_as_typed_text() {
    assert_eq!(ass_text("Hello there"), "Hello there");
    assert_eq!(ass_text("two\nlines"), r"two\Nlines");
    assert_eq!(ass_text("cr\rlf\r\nend"), r"cr\Nlf\Nend");
    assert_eq!(
        ass_text(r"{\c&H0000FF&}red"),
        "\\{\\\u{2060}c&H0000FF&\\}red"
    );
    assert_eq!(ass_text(r"C:\New"), "C:\\\u{2060}New");
    // No line of the result can start a new ASS line.
    let injected = ass_text("x\r[Fonts]\rfontname: a.ttf\rDialogue: 0,0:00:00.00,9:00:00.00,Default,,0,0,0,,{\\p1}m 0 0 l 9 9");
    assert!(!injected.contains('\r') && !injected.contains('\n'));
    assert!(!injected.contains("{\\p1}"));
}

#[test]
fn ass_field_refuses_separators_with_a_reason() {
    assert!(ass_field("style name", "Big Yellow-2").is_ok());
    assert!(ass_field("font", "Noto Sans CJK JP").is_ok());
    for bad in ["a,b", "a\nb", "a\rStyle: x", "a\u{0}b"] {
        let err = ass_field("font", bad).unwrap_err();
        assert_eq!(err.code, error_codes::INVALID_ARGS);
        assert!(err.suggested_action.is_some());
    }
}
