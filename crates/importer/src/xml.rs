//! Schlanker XML-Baum über `quick-xml` (für tolerantes Lesen fremder Exporte).

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::ImportError;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct XmlNode {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<XmlNode>,
    pub text: String,
}

impl XmlNode {
    pub fn child(&self, name: &str) -> Option<&XmlNode> {
        self.children.iter().find(|c| c.name.eq_ignore_ascii_case(name))
    }

    /// Text eines Kindelements (leer, wenn nicht vorhanden).
    pub fn get(&self, name: &str) -> Option<&str> {
        self.child(name).map(|c| c.text.as_str()).filter(|t| !t.is_empty())
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

fn utf8(b: impl AsRef<[u8]>, what: &str, pos: u64) -> Result<String, ImportError> {
    std::str::from_utf8(b.as_ref())
        .map(str::to_string)
        .map_err(|e| ImportError::Encoding(format!("{what} bei Byte {}: ungültiges UTF-8 ({e})", pos)))
}

/// Liest ein XML-Dokument in einen Baum.
pub fn parse(text: &str) -> Result<XmlNode, ImportError> {
    let mut r = Reader::from_str(text);
    let mut stack: Vec<XmlNode> = vec![XmlNode { name: "#document".into(), ..Default::default() }];
    loop {
        let pos = r.buffer_position();
        let ev = r.read_event().map_err(|e| ImportError::Format(format!("XML bei Byte {pos}: {e}")))?;
        match ev {
            Event::Start(e) => {
                let name = utf8(e.name().as_ref(), "Elementname", pos)?;
                let mut attrs = Vec::new();
                for a in e.attributes().flatten() {
                    attrs.push((utf8(a.key.as_ref(), "Attribut", pos)?, unescape(&utf8(a.value.as_bytes(), "Attributwert", pos)?)));
                }
                stack.push(XmlNode { name, attrs, ..Default::default() });
            }
            Event::Empty(e) => {
                let name = utf8(e.name().as_ref(), "Elementname", pos)?;
                let mut attrs = Vec::new();
                for a in e.attributes().flatten() {
                    attrs.push((utf8(a.key.as_ref(), "Attribut", pos)?, unescape(&utf8(a.value.as_bytes(), "Attributwert", pos)?)));
                }
                stack.last_mut().expect("Wurzel").children.push(XmlNode { name, attrs, ..Default::default() });
            }
            Event::End(_) => {
                if stack.len() > 1 {
                    let n = stack.pop().expect("vorhanden");
                    stack.last_mut().expect("Wurzel").children.push(n);
                }
            }
            Event::Text(t) => {
                let s = utf8(t.as_ref(), "Text", pos)?;
                stack.last_mut().expect("Wurzel").text.push_str(&s);
            }
            Event::CData(c) => {
                let s = utf8(c.as_ref(), "CDATA", pos)?;
                stack.last_mut().expect("Wurzel").text.push_str(&s);
            }
            Event::GeneralRef(g) => {
                let s = match g.resolve_char_ref() {
                    Ok(Some(c)) => c.to_string(),
                    _ => match g.as_ref() {
                        "amp" => "&".into(),
                        "lt" => "<".into(),
                        "gt" => ">".into(),
                        "quot" => "\"".into(),
                        "apos" => "'".into(),
                        other => format!("&{other};"),
                    },
                };
                stack.last_mut().expect("Wurzel").text.push_str(&s);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    while stack.len() > 1 {
        let n = stack.pop().expect("vorhanden");
        stack.last_mut().expect("Wurzel").children.push(n);
    }
    let mut doc = stack.pop().expect("Wurzel");
    // Leerraum um Elementtexte entfernen (nicht innerhalb).
    fn trim(n: &mut XmlNode) {
        if !n.children.is_empty() && n.text.trim().is_empty() {
            n.text.clear();
        }
        for c in &mut n.children {
            trim(c);
        }
    }
    trim(&mut doc);
    Ok(doc)
}
