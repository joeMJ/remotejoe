//! Tabs, Kacheln und abgerissene Fenster.
//!
//! Ein Tab enthält eine oder mehrere Sitzungen. Die Anordnung ist 1×1,
//! 1×2, 2×2 oder frei (Teiler je Sitzung). Ein Tab kann in ein eigenes
//! Fenster verschoben werden.

use serde::Serialize;

pub type SessionId = u64;
pub type TabId = u64;
pub type WindowId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Arrangement {
    Single,
    OneByTwo,
    TwoByTwo,
    Free,
}

impl Arrangement {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "single" => Arrangement::Single,
            "one_by_two" => Arrangement::OneByTwo,
            "two_by_two" => Arrangement::TwoByTwo,
            "free" => Arrangement::Free,
            _ => return None,
        })
    }

    /// Passende Anordnung für `n` Sitzungen.
    pub fn for_count(n: usize) -> Self {
        match n {
            0 | 1 => Arrangement::Single,
            2 => Arrangement::OneByTwo,
            3 | 4 => Arrangement::TwoByTwo,
            _ => Arrangement::Free,
        }
    }
}

/// Zelle im Raster: Zeile, Spalte, Zeilen- und Spaltenspanne.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Cell {
    pub row: u32,
    pub col: u32,
    pub row_span: u32,
    pub col_span: u32,
}

/// Rasterzellen für `n` Sitzungen in einer Anordnung.
pub fn cells(a: Arrangement, n: usize) -> Vec<Cell> {
    let c = |row, col, row_span, col_span| Cell { row, col, row_span, col_span };
    match (a, n) {
        (_, 0) => vec![],
        (Arrangement::Single, _) | (_, 1) => vec![c(0, 0, 1, 1)],
        (Arrangement::OneByTwo, _) => (0..n as u32).map(|i| c(0, i, 1, 1)).collect(),
        (Arrangement::TwoByTwo, 2) => vec![c(0, 0, 1, 1), c(0, 1, 1, 1)],
        (Arrangement::TwoByTwo, 3) => vec![c(0, 0, 1, 1), c(0, 1, 1, 1), c(1, 0, 1, 2)],
        (Arrangement::TwoByTwo, _) => (0..n as u32).map(|i| c(i / 2, i % 2, 1, 1)).collect(),
        (Arrangement::Free, _) => {
            let cols = (n as f64).sqrt().ceil() as u32;
            (0..n as u32).map(|i| c(i / cols, i % cols, 1, 1)).collect()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Tab {
    pub id: TabId,
    pub title: String,
    pub sessions: Vec<SessionId>,
    pub arrangement: Arrangement,
    /// Bei der Einzelanordnung mit mehreren Sitzungen: sichtbare Sitzung.
    pub focus: Option<SessionId>,
    /// Teilerpositionen (Anteile) je Zeile für die freie Anordnung.
    pub ratios: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Window {
    pub id: WindowId,
    pub tabs: Vec<TabId>,
    pub active: Option<TabId>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Workspace {
    pub windows: Vec<Window>,
    pub tabs: Vec<Tab>,
    next: u64,
}

impl Default for Workspace {
    fn default() -> Self {
        Workspace { windows: vec![Window { id: 0, tabs: Vec::new(), active: None }], tabs: Vec::new(), next: 1 }
    }
}

impl Workspace {
    fn id(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    pub fn tab(&self, id: TabId) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    fn tab_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    fn window_of(&self, tab: TabId) -> Option<usize> {
        self.windows.iter().position(|w| w.tabs.contains(&tab))
    }

    pub fn tab_of(&self, s: SessionId) -> Option<TabId> {
        self.tabs.iter().find(|t| t.sessions.contains(&s)).map(|t| t.id)
    }

    /// Neue Sitzung in einem neuen Tab im Hauptfenster.
    pub fn open(&mut self, s: SessionId, title: &str) -> TabId {
        let id = self.id();
        self.tabs.push(Tab { id, title: title.into(), sessions: vec![s], arrangement: Arrangement::Single, focus: Some(s), ratios: vec![] });
        self.windows[0].tabs.push(id);
        self.windows[0].active = Some(id);
        id
    }

    /// Sitzung in einen vorhandenen Tab kacheln (verlässt ihren bisherigen Tab).
    pub fn tile(&mut self, s: SessionId, into: TabId) -> bool {
        if self.tab(into).is_none() || self.tab(into).is_some_and(|t| t.sessions.contains(&s)) {
            return false;
        }
        self.remove_session(s);
        let Some(t) = self.tab_mut(into) else { return false };
        t.sessions.push(s);
        t.arrangement = Arrangement::for_count(t.sessions.len());
        t.title = format!("{} Sitzungen", t.sessions.len());
        t.ratios.clear();
        true
    }

    pub fn set_arrangement(&mut self, tab: TabId, a: Arrangement) -> bool {
        match self.tab_mut(tab) {
            Some(t) => {
                t.arrangement = a;
                t.ratios.clear();
                true
            }
            None => false,
        }
    }

    pub fn set_ratios(&mut self, tab: TabId, ratios: Vec<f32>) -> bool {
        match self.tab_mut(tab) {
            Some(t) if ratios.iter().all(|r| (0.0..=1.0).contains(r)) => {
                t.arrangement = Arrangement::Free;
                t.ratios = ratios;
                true
            }
            _ => false,
        }
    }

    /// Sitzung aus ihrem Tab lösen; leere Tabs verschwinden.
    pub fn remove_session(&mut self, s: SessionId) {
        if let Some(tid) = self.tab_of(s) {
            let empty = {
                let t = self.tab_mut(tid).expect("vorhanden");
                t.sessions.retain(|x| *x != s);
                if t.focus == Some(s) {
                    t.focus = t.sessions.first().copied();
                }
                t.arrangement = Arrangement::for_count(t.sessions.len());
                t.sessions.is_empty()
            };
            if empty {
                self.close_tab(tid);
            }
        }
    }

    pub fn close_tab(&mut self, tab: TabId) {
        self.tabs.retain(|t| t.id != tab);
        for w in &mut self.windows {
            w.tabs.retain(|t| *t != tab);
            if w.active == Some(tab) {
                w.active = w.tabs.last().copied();
            }
        }
        // Leere Nebenfenster schließen, das Hauptfenster bleibt.
        let mut i = 1;
        while i < self.windows.len() {
            if self.windows[i].tabs.is_empty() {
                self.windows.remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// Tab in ein eigenes Fenster verschieben.
    pub fn detach(&mut self, tab: TabId) -> Option<WindowId> {
        let wi = self.window_of(tab)?;
        if self.windows[wi].tabs.len() == 1 && wi != 0 {
            return Some(self.windows[wi].id);
        }
        self.windows[wi].tabs.retain(|t| *t != tab);
        if self.windows[wi].active == Some(tab) {
            self.windows[wi].active = self.windows[wi].tabs.last().copied();
        }
        let id = self.id();
        self.windows.push(Window { id, tabs: vec![tab], active: Some(tab) });
        Some(id)
    }

    /// Tab zurück ins Hauptfenster.
    pub fn attach(&mut self, tab: TabId) -> bool {
        let Some(wi) = self.window_of(tab) else { return false };
        if wi == 0 {
            return false;
        }
        self.windows[wi].tabs.retain(|t| *t != tab);
        self.windows[0].tabs.push(tab);
        self.windows[0].active = Some(tab);
        self.close_tab_if_window_empty();
        true
    }

    fn close_tab_if_window_empty(&mut self) {
        let mut i = 1;
        while i < self.windows.len() {
            if self.windows[i].tabs.is_empty() {
                self.windows.remove(i);
            } else {
                i += 1;
            }
        }
    }

    pub fn activate(&mut self, tab: TabId) {
        if let Some(wi) = self.window_of(tab) {
            self.windows[wi].active = Some(tab);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_cells() {
        assert_eq!(cells(Arrangement::OneByTwo, 2).len(), 2);
        assert_eq!(cells(Arrangement::TwoByTwo, 3)[2], Cell { row: 1, col: 0, row_span: 1, col_span: 2 });
        assert_eq!(cells(Arrangement::TwoByTwo, 4)[3], Cell { row: 1, col: 1, row_span: 1, col_span: 1 });
        let free = cells(Arrangement::Free, 5);
        assert_eq!(free.len(), 5);
        assert_eq!(free[4], Cell { row: 1, col: 1, row_span: 1, col_span: 1 });
        assert_eq!(cells(Arrangement::Single, 3).len(), 1, "Einzelanordnung zeigt eine Sitzung");
    }

    #[test]
    fn open_tile_detach_close() {
        let mut w = Workspace::default();
        let t1 = w.open(1, "srv1");
        let t2 = w.open(2, "srv2");
        let _t3 = w.open(3, "srv3");
        assert_eq!(w.windows[0].tabs.len(), 3);
        assert!(w.tile(2, t1));
        assert_eq!(w.tab(t1).unwrap().sessions, vec![1, 2]);
        assert_eq!(w.tab(t1).unwrap().arrangement, Arrangement::OneByTwo);
        assert!(w.tab(t2).is_none(), "leerer Tab verschwindet");
        assert!(w.tile(3, t1));
        assert_eq!(w.tab(t1).unwrap().arrangement, Arrangement::TwoByTwo);
        assert!(!w.tile(3, t1), "doppelt kacheln geht nicht");
        assert!(w.set_ratios(t1, vec![0.3, 0.7]));
        assert_eq!(w.tab(t1).unwrap().arrangement, Arrangement::Free);
        // Abreißen und zurück.
        let t4 = w.open(4, "srv4");
        let win = w.detach(t4).unwrap();
        assert_eq!(w.windows.len(), 2);
        assert_eq!(w.windows[1].id, win);
        assert!(!w.windows[0].tabs.contains(&t4));
        assert!(w.attach(t4));
        assert_eq!(w.windows.len(), 1);
        // Sitzung schließen.
        w.remove_session(1);
        assert_eq!(w.tab(t1).unwrap().sessions, vec![2, 3]);
        assert_eq!(w.tab(t1).unwrap().focus, Some(2));
        let t5 = w.open(5, "x");
        w.detach(t5);
        w.remove_session(5);
        assert_eq!(w.windows.len(), 1, "leeres Nebenfenster wird geschlossen");
    }
}
