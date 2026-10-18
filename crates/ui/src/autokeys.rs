//! Ablauf der Tastenfolge nach dem Verbinden: Auslöser abwarten, dann Schritt für Schritt senden.
//!
//! Reine Zustandsmaschine ohne Zugriff auf Sitzungen (gut testbar); `sessions.rs` ruft `poll` regelmäßig auf
//! und sendet die zurückgegebenen Schritte an den Worker.

use std::time::{Duration, Instant};

use rj_core::autokeys::{AutoKeys, AutoTrigger, KeyStep};

/// Lauf einer Tastenfolge.
pub struct AutoRun {
    steps: Vec<KeyStep>,
    trigger: AutoTrigger,
    wait: Duration,
    max: Duration,
    started: Instant,
    ready: bool,
    idx: usize,
    resume_at: Instant,
}

impl AutoRun {
    /// `now`: Zeitpunkt, zu dem die Verbindung stand.
    pub fn new(cfg: &AutoKeys, steps: Vec<KeyStep>, now: Instant) -> AutoRun {
        AutoRun {
            steps,
            trigger: cfg.trigger,
            wait: Duration::from_millis(cfg.wait_ms as u64),
            max: Duration::from_millis(cfg.max_ms as u64),
            started: now,
            ready: false,
            idx: 0,
            resume_at: now,
        }
    }

    /// Läuft schon (Auslöser ist erreicht)?
    pub fn is_running(&self) -> bool {
        self.ready
    }

    /// Liefert die jetzt zu sendenden Schritte und ob die Folge damit zu Ende ist.
    /// `last_activity`: Zeitpunkt des letzten Bild- bzw. Ausgabeereignisses der Sitzung.
    pub fn poll(&mut self, now: Instant, last_activity: Instant) -> (Vec<KeyStep>, bool) {
        if !self.ready {
            let go = match self.trigger {
                AutoTrigger::Delay => now >= self.started + self.wait,
                AutoTrigger::Quiet => now >= last_activity.max(self.started) + self.wait || now >= self.started + self.max,
            };
            if !go {
                return (Vec::new(), false);
            }
            self.ready = true;
            self.resume_at = now;
        }
        let mut out = Vec::new();
        while self.idx < self.steps.len() {
            if now < self.resume_at {
                break;
            }
            match &self.steps[self.idx] {
                KeyStep::Pause(ms) => {
                    self.resume_at = now + Duration::from_millis(*ms as u64);
                    self.idx += 1;
                    // Nach einer Pause erst beim nächsten Aufruf weiter.
                    break;
                }
                s => {
                    out.push(s.clone());
                    self.idx += 1;
                }
            }
        }
        let done = self.idx >= self.steps.len() && now >= self.resume_at;
        (out, done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rj_core::autokeys::parse_script;

    fn run(trigger: AutoTrigger, wait_ms: u32, max_ms: u32, script: &str, t0: Instant) -> AutoRun {
        let cfg = AutoKeys { enabled: true, script: script.into(), trigger, wait_ms, max_ms, ..Default::default() };
        AutoRun::new(&cfg, parse_script(script).unwrap(), t0)
    }

    #[test]
    fn delay_waits_then_sends_everything_with_pauses() {
        let t0 = Instant::now();
        let mut r = run(AutoTrigger::Delay, 2000, 0, "Taste Enter\nPause 500\nText x\nTaste Enter", t0);
        let ms = |n| t0 + Duration::from_millis(n);
        assert_eq!(r.poll(ms(1999), t0), (vec![], false), "vor Ablauf der Wartezeit nichts");
        let (a, done) = r.poll(ms(2000), t0);
        assert_eq!(a.len(), 1, "bis zur Pause");
        assert!(!done);
        assert_eq!(r.poll(ms(2400), t0), (vec![], false), "Pause läuft");
        let (b, done) = r.poll(ms(2500), t0);
        assert_eq!(b.len(), 2);
        assert!(done);
    }

    #[test]
    fn quiet_needs_calm_or_falls_back_to_the_latest_time() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        // Ständige Bewegung: erst nach `max` wird gesendet.
        let mut r = run(AutoTrigger::Quiet, 800, 5000, "Taste Enter", t0);
        for n in (0..5000).step_by(300) {
            assert_eq!(r.poll(ms(n), ms(n)).0.len(), 0, "bei Bewegung (t={n}) nichts");
        }
        assert_eq!(r.poll(ms(5000), ms(4900)).0.len(), 1, "spätestens nach max");
        // Letzte Bewegung bei 300 ms: Ruhe erst ab 1100 ms.
        let mut r = run(AutoTrigger::Quiet, 800, 20_000, "Taste Enter", t0);
        assert_eq!(r.poll(ms(1000), ms(300)).0.len(), 0, "Ruhe erst seit 700 ms");
        let (a, done) = r.poll(ms(1100), ms(300));
        assert_eq!(a.len(), 1);
        assert!(done);
    }

    #[test]
    fn empty_script_finishes_at_once_and_pause_only_ends_after_it() {
        let t0 = Instant::now();
        let mut r = run(AutoTrigger::Delay, 0, 0, "# nichts", t0);
        assert_eq!(r.poll(t0, t0), (vec![], true));
        let mut r = run(AutoTrigger::Delay, 0, 0, "Pause 300", t0);
        assert_eq!(r.poll(t0, t0), (vec![], false));
        assert_eq!(r.poll(t0 + Duration::from_millis(300), t0), (vec![], true));
    }
}
