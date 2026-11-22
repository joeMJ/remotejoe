//! Sperre nach Leerlauf und Erkennung des Aufwachens aus dem Standby.
//!
//! Leerlauf wird mit der monotonen Uhr gemessen; sie steht unter Linux
//! während des Standby still, sodass die Standby-Zeit nicht als Leerlauf
//! zählt. Zusätzlich wird ein Sprung der Systemuhr gegenüber der monotonen
//! Uhr als Aufwachen gewertet (Rückfall, wenn die Meldung des Systems fehlt).
//! Nach dem Aufwachen wird nie gesperrt; der Leerlaufzeitgeber startet neu.

use std::time::{Duration, Instant, SystemTime};

/// Ab dieser Abweichung zwischen Systemuhr und monotoner Uhr gilt ein
/// Abschnitt als Standby.
pub const JUMP: Duration = Duration::from_secs(20);

/// Ergebnis eines Takts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleEvent {
    None,
    /// Leerlaufgrenze erreicht: jetzt sperren.
    Lock,
    /// Aufwachen erkannt (Sprung der Systemuhr).
    Resumed,
}

#[derive(Debug, Clone)]
pub struct IdleWatch {
    limit: Option<Duration>,
    last_activity: Instant,
    last_mono: Instant,
    last_wall: SystemTime,
    /// Bereits gesperrt (kein erneutes Auslösen bis zur nächsten Aktivität).
    fired: bool,
}

impl IdleWatch {
    /// `minutes = 0` schaltet die Sperre ab.
    pub fn new(minutes: u32, now: Instant, wall: SystemTime) -> Self {
        IdleWatch { limit: limit(minutes), last_activity: now, last_mono: now, last_wall: wall, fired: false }
    }

    pub fn set_minutes(&mut self, minutes: u32, now: Instant) {
        self.limit = limit(minutes);
        self.activity(now);
    }

    pub fn enabled(&self) -> bool {
        self.limit.is_some()
    }

    /// Eingabe des Benutzers.
    pub fn activity(&mut self, now: Instant) {
        self.last_activity = now;
        self.fired = false;
    }

    /// Meldung des Systems über das Aufwachen: Zeitgeber neu starten.
    pub fn resumed(&mut self, now: Instant, wall: SystemTime) {
        self.last_activity = now;
        self.last_mono = now;
        self.last_wall = wall;
        self.fired = false;
    }

    /// Leerlauf bis jetzt.
    pub fn idle_for(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.last_activity)
    }

    /// Regelmäßig aufrufen (z. B. aus dem Takt der Oberfläche).
    pub fn tick(&mut self, now: Instant, wall: SystemTime) -> IdleEvent {
        let mono = now.saturating_duration_since(self.last_mono);
        let wall_delta = wall.duration_since(self.last_wall).unwrap_or(Duration::ZERO);
        self.last_mono = now;
        self.last_wall = wall;
        if wall_delta > mono + JUMP {
            // Die Systemuhr ist weitergelaufen, die monotone nicht: Standby.
            self.resumed(now, wall);
            return IdleEvent::Resumed;
        }
        match self.limit {
            Some(l) if !self.fired && self.idle_for(now) >= l => {
                self.fired = true;
                IdleEvent::Lock
            }
            _ => IdleEvent::None,
        }
    }
}

fn limit(minutes: u32) -> Option<Duration> {
    (minutes > 0).then(|| Duration::from_secs(minutes as u64 * 60))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_by_default_never_locks() {
        let t0 = Instant::now();
        let w0 = SystemTime::now();
        let mut w = IdleWatch::new(0, t0, w0);
        assert!(!w.enabled());
        assert_eq!(w.tick(t0 + Duration::from_secs(36_000), w0 + Duration::from_secs(36_000)), IdleEvent::None);
    }

    #[test]
    fn locks_after_limit_once_and_activity_resets() {
        let t0 = Instant::now();
        let w0 = SystemTime::now();
        let mut w = IdleWatch::new(5, t0, w0);
        let at = |s: u64| (t0 + Duration::from_secs(s), w0 + Duration::from_secs(s));
        let (t, wl) = at(299);
        assert_eq!(w.tick(t, wl), IdleEvent::None);
        let (t, wl) = at(300);
        assert_eq!(w.tick(t, wl), IdleEvent::Lock);
        let (t, wl) = at(301);
        assert_eq!(w.tick(t, wl), IdleEvent::None, "nur einmal");
        w.activity(t);
        let (t, wl) = at(500);
        assert_eq!(w.tick(t, wl), IdleEvent::None);
        let (t, wl) = at(601);
        assert_eq!(w.tick(t, wl), IdleEvent::Lock);
    }

    #[test]
    fn standby_time_does_not_count_and_wake_never_locks() {
        let t0 = Instant::now();
        let w0 = SystemTime::now();
        let mut w = IdleWatch::new(5, t0, w0);
        // 4 Minuten Leerlauf, dann 2 Stunden Standby: die monotone Uhr steht,
        // die Systemuhr springt.
        let t1 = t0 + Duration::from_secs(240);
        assert_eq!(w.tick(t1, w0 + Duration::from_secs(240)), IdleEvent::None);
        let t2 = t1 + Duration::from_secs(1);
        assert_eq!(w.tick(t2, w0 + Duration::from_secs(240 + 7200)), IdleEvent::Resumed);
        assert_eq!(w.idle_for(t2), Duration::ZERO, "Zeitgeber neu gestartet");
        // Nach dem Aufwachen läuft die volle Frist neu.
        assert_eq!(w.tick(t2 + Duration::from_secs(299), w0 + Duration::from_secs(240 + 7200 + 299)), IdleEvent::None);
        assert_eq!(w.tick(t2 + Duration::from_secs(300), w0 + Duration::from_secs(240 + 7200 + 300)), IdleEvent::Lock);
    }

    #[test]
    fn system_resume_message_resets_timer() {
        let t0 = Instant::now();
        let w0 = SystemTime::now();
        let mut w = IdleWatch::new(1, t0, w0);
        let t1 = t0 + Duration::from_secs(59);
        w.resumed(t1, w0 + Duration::from_secs(59));
        assert_eq!(w.tick(t1 + Duration::from_secs(30), w0 + Duration::from_secs(89)), IdleEvent::None);
        // Kleine Abweichungen der Uhren gelten nicht als Standby.
        assert_eq!(w.tick(t1 + Duration::from_secs(31), w0 + Duration::from_secs(100)), IdleEvent::None);
    }
}
