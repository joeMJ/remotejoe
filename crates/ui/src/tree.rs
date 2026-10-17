//! Baumdarstellung des Index für die Oberfläche: Reihenfolge bleibt
//! erhalten, Namen werden mit sichtbar markierten Sonderzeichen gezeigt,
//! die Suche behält die Elternordner der Treffer.

use rj_core::charset::mark_visible;
use rj_core::{Folder, Index, Node, Protocol};
use serde::Serialize;
use uuid::Uuid;

/// Ampel für SNMP-Zustand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Light {
    Grey,
    Green,
    Yellow,
    Red,
}

/// Knoten für die Oberfläche.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UiNode {
    pub id: Uuid,
    /// Anzeigename mit Markierung auffälliger Zeichen (`⟦U+200B⟧`).
    pub name: String,
    /// Name enthält auffällige Zeichen.
    pub suspicious: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<Protocol>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub light: Option<Light>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<UiNode>,
    pub folder: bool,
}

fn matches(name: &str, host: Option<&str>, q: &[String]) -> bool {
    let hay = format!("{} {}", name.to_lowercase(), host.unwrap_or("").to_lowercase());
    q.iter().all(|w| hay.contains(w.as_str()))
}

fn build(f: &Folder, q: &[String], lights: &dyn Fn(Uuid) -> Option<Light>) -> Vec<UiNode> {
    let mut out = Vec::new();
    for n in &f.children {
        match n {
            Node::Folder(sub) => {
                let children = build(sub, q, lights);
                let hit = matches(&sub.name, None, q);
                if q.is_empty() || hit || !children.is_empty() {
                    // Bei einem Ordnertreffer alle Kinder zeigen.
                    let children = if hit && !q.is_empty() { build(sub, &[], lights) } else { children };
                    out.push(UiNode {
                        id: sub.id,
                        name: mark_visible(&sub.name, false).into_owned(),
                        suspicious: mark_visible(&sub.name, false) != sub.name,
                        protocol: None,
                        host: None,
                        light: lights(sub.id),
                        children,
                        folder: true,
                    });
                }
            }
            Node::Target(t) => {
                if q.is_empty() || matches(&t.name, Some(&t.host), q) {
                    let display = mark_visible(&t.name, false);
                    out.push(UiNode {
                        id: t.id,
                        suspicious: display != t.name,
                        name: display.into_owned(),
                        protocol: Some(t.protocol),
                        host: Some(mark_visible(&t.host, false).into_owned()),
                        light: lights(t.id),
                        children: Vec::new(),
                        folder: false,
                    });
                }
            }
        }
    }
    out
}

/// Baum mit Filter (Leerzeichen-getrennte Begriffe, alle müssen passen).
pub fn tree(index: &Index, filter: &str, lights: &dyn Fn(Uuid) -> Option<Light>) -> Vec<UiNode> {
    let q: Vec<String> = filter.split_whitespace().map(|w| w.to_lowercase()).collect();
    build(&index.root, &q, lights)
}

/// Flache Liste für eine Ansicht oder die Favoriten (Reihenfolge der Liste;
/// Ordner mit ihrem Inhalt).
pub fn listed(index: &Index, ids: &[Uuid], filter: &str, lights: &dyn Fn(Uuid) -> Option<Light>) -> Vec<UiNode> {
    let q: Vec<String> = filter.split_whitespace().map(|w| w.to_lowercase()).collect();
    let mut out = Vec::new();
    for id in ids {
        let Some(node) = index.path_to(*id).and_then(|p| p.last().copied()) else { continue };
        let mut tmp = Folder::new("");
        tmp.children.push(node.clone());
        out.extend(build(&tmp, &q, lights));
    }
    out
}

pub fn count(nodes: &[UiNode]) -> usize {
    nodes.iter().map(|n| 1 + count(&n.children)).sum()
}

/// Erfundener Index für Vorführung und Tests (keine echten Namen).
pub fn demo_index(targets: usize) -> Index {
    let mut idx = Index::default();
    let sites = ["Standort Nord", "Standort Süd", "Rechenzentrum", "Büro Ost 🏢", "Labor"];
    let protos = [Protocol::Rdp, Protocol::Ssh, Protocol::Vnc, Protocol::Web, Protocol::Psrp, Protocol::Proxmox, Protocol::Hyperv];
    let per = targets.div_ceil(sites.len() * 4).max(1);
    let mut n = 0;
    'outer: for (si, s) in sites.iter().enumerate() {
        let mut site = Folder::new(*s);
        for g in 0..4 {
            let mut group = Folder::new(["Server", "Clients", "Netz", "Speicher"][g]);
            for i in 0..per {
                if n >= targets {
                    site.children.push(Node::Folder(group));
                    idx.root.children.push(Node::Folder(site));
                    break 'outer;
                }
                let p = protos[n % protos.len()];
                let name = if n == 7 { "versteckt\u{200B}name".to_string() } else { format!("{}-{:03}", p.as_str(), i) };
                let t = rj_core::Target::new(name, p, format!("h{n}.site{si}.example.test"));
                group.children.push(Node::Target(t));
                n += 1;
            }
            site.children.push(Node::Folder(group));
        }
        idx.root.children.push(Node::Folder(site));
    }
    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_and_marking() {
        let idx = demo_index(40);
        let t = tree(&idx, "", &|_| None);
        assert_eq!(t[0].name, "Standort Nord");
        assert_eq!(t[3].name, "Büro Ost 🏢");
        let all = serde_json::to_string(&t).unwrap();
        assert!(all.contains("versteckt⟦U+200B⟧name"));
        assert_eq!(count(&t), 40 + 20 + 5, "40 Ziele, 20 Gruppen, 5 Standorte");
    }

    #[test]
    fn filter_keeps_parents() {
        let idx = demo_index(100);
        let t = tree(&idx, "ssh-000", &|_| None);
        assert!(!t.is_empty());
        fn leaves(n: &[UiNode], out: &mut Vec<String>) {
            for x in n {
                if x.folder {
                    leaves(&x.children, out)
                } else {
                    out.push(x.name.clone())
                }
            }
        }
        let mut l = Vec::new();
        leaves(&t, &mut l);
        assert!(l.iter().all(|n| n == "ssh-000"), "{l:?}");
        // Suche nach Host und mehreren Begriffen.
        let t = tree(&idx, "site1 rdp", &|_| None);
        let mut l = Vec::new();
        leaves(&t, &mut l);
        assert!(!l.is_empty() && l.iter().all(|n| n.starts_with("rdp")));
        // Ordnertreffer zeigt alle Kinder.
        let t = tree(&idx, "labor", &|_| None);
        assert_eq!(t.len(), 1);
        assert!(count(&t) > 4);
    }

    #[test]
    fn five_hundred_entries_are_fast() {
        let idx = demo_index(500);
        let start = std::time::Instant::now();
        let t = tree(&idx, "", &|_| Some(Light::Green));
        let json = serde_json::to_string(&t).unwrap();
        let el = start.elapsed();
        let mut targets = 0;
        idx.for_each_target(|_| targets += 1);
        assert_eq!(targets, 500);
        assert!(el.as_millis() < 200, "Baum für 500 Einträge in {el:?}");
        assert!(json.len() > 10_000);
    }
}
