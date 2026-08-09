//! Minimal XML writer with mandatory escaping (the export mapping requires a
//! writer that escapes attribute/text content"). Hand-rolled instead of
//! quick-xml: ~60 lines buys exact byte-level control over the proven-to-
//! import shapes in examples/ (2-space indent, ` />` self-close, single-quote
//! declaration) with zero new dependencies. Correctness lives in [`esc`].

/// Escape XML-special characters for both attribute and text content.
/// One escaper for both contexts — over-escaping text with &quot;/&apos; is
/// harmless and removes a whole class of context-confusion bugs.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Streaming document builder. Caller is responsible for balanced
/// open/close pairs (the serializers are short and fully unit-tested against
/// a real parser, which catches imbalance immediately).
pub struct Xml {
    out: String,
    depth: usize,
}

impl Xml {
    /// New document with the declaration every example file uses.
    pub fn new() -> Self {
        Self {
            out: String::from("<?xml version='1.0' encoding='utf-8'?>\n"),
            depth: 0,
        }
    }

    fn pad(&mut self) {
        for _ in 0..self.depth {
            self.out.push_str("  ");
        }
    }

    fn write_attrs(&mut self, attrs: &[(&str, &str)]) {
        for (k, v) in attrs {
            self.out.push(' ');
            self.out.push_str(k);
            self.out.push_str("=\"");
            self.out.push_str(&esc(v));
            self.out.push('"');
        }
    }

    /// `<tag a="v">` + indent in.
    pub fn open(&mut self, tag: &str, attrs: &[(&str, &str)]) {
        self.pad();
        self.out.push('<');
        self.out.push_str(tag);
        self.write_attrs(attrs);
        self.out.push_str(">\n");
        self.depth += 1;
    }

    /// Indent out + `</tag>`.
    pub fn close(&mut self, tag: &str) {
        self.depth = self.depth.saturating_sub(1);
        self.pad();
        self.out.push_str("</");
        self.out.push_str(tag);
        self.out.push_str(">\n");
    }

    /// Self-closing `<tag a="v" />` (space before slash, matching examples/).
    pub fn leaf(&mut self, tag: &str, attrs: &[(&str, &str)]) {
        self.pad();
        self.out.push('<');
        self.out.push_str(tag);
        self.write_attrs(attrs);
        self.out.push_str(" />\n");
    }

    /// `<tag a="v">text</tag>` on one line. Empty text yields `<tag></tag>` —
    /// exactly the blank `<duration></duration>` Resolve requires in xmeml
    /// file defs, which a self-closing form would NOT satisfy
    /// byte-shape-wise.
    pub fn text_el(&mut self, tag: &str, attrs: &[(&str, &str)], text: &str) {
        self.pad();
        self.out.push('<');
        self.out.push_str(tag);
        self.write_attrs(attrs);
        self.out.push('>');
        self.out.push_str(&esc(text));
        self.out.push_str("</");
        self.out.push_str(tag);
        self.out.push_str(">\n");
    }

    /// Finish and return the document text.
    pub fn finish(self) -> String {
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_attr_and_text() {
        let mut x = Xml::new();
        x.open("a", &[("p", "x<>&\"'y")]);
        x.text_el("b", &[], "1 < 2 & 3");
        x.close("a");
        let s = x.finish();
        assert!(s.contains("p=\"x&lt;&gt;&amp;&quot;&apos;y\""));
        assert!(s.contains("<b>1 &lt; 2 &amp; 3</b>"));
    }
}
