//! Farbschemata hell und dunkel mit geprüftem Kontrast (WCAG 2.1 AA).

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Palette {
    pub window: &'static str,
    pub base: &'static str,
    pub alternate: &'static str,
    pub text: &'static str,
    pub muted: &'static str,
    pub highlight: &'static str,
    pub highlight_text: &'static str,
    pub button: &'static str,
    pub link: &'static str,
    pub warning: &'static str,
    pub error: &'static str,
    pub ok: &'static str,
}

pub const LIGHT: Palette = Palette {
    window: "#f3f3f3",
    base: "#ffffff",
    alternate: "#f0f4f8",
    text: "#1b1b1b",
    muted: "#555555",
    highlight: "#0b5cad",
    highlight_text: "#ffffff",
    button: "#e6e6e6",
    link: "#0b5cad",
    warning: "#8a5300",
    error: "#b00020",
    ok: "#1b6e20",
};

pub const DARK: Palette = Palette {
    window: "#1f1f1f",
    base: "#141414",
    alternate: "#1b2229",
    text: "#ececec",
    muted: "#b0b0b0",
    highlight: "#3d8fe0",
    highlight_text: "#000000",
    button: "#2d2d2d",
    link: "#7cb8f2",
    warning: "#f0b04a",
    error: "#ff8a8a",
    ok: "#7ed387",
};

fn channel(hex: &str, i: usize) -> f64 {
    let v = u8::from_str_radix(&hex[1 + i * 2..3 + i * 2], 16).unwrap_or(0) as f64 / 255.0;
    if v <= 0.03928 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// Relative Leuchtdichte nach WCAG.
pub fn luminance(hex: &str) -> f64 {
    0.2126 * channel(hex, 0) + 0.7152 * channel(hex, 1) + 0.0722 * channel(hex, 2)
}

/// Kontrastverhältnis (1 … 21).
pub fn contrast(a: &str, b: &str) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// Dunkles Schema nach Systemvorgabe? Linux: GNOME-Einstellung bzw.
/// `GTK_THEME`; Windows: Registrierung (`AppsUseLightTheme`).
pub fn system_prefers_dark() -> bool {
    if let Ok(v) = std::env::var("REMOTEJOE_THEME") {
        return v == "dark";
    }
    #[cfg(windows)]
    {
        let out = std::process::Command::new("reg")
            .args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "/v", "AppsUseLightTheme"])
            .output();
        if let Ok(o) = out {
            return String::from_utf8_lossy(&o.stdout).contains("0x0");
        }
        false
    }
    #[cfg(not(windows))]
    {
        if std::env::var("GTK_THEME").is_ok_and(|t| t.to_lowercase().contains("dark")) {
            return true;
        }
        std::process::Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "color-scheme"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("dark"))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contrast_meets_wcag_aa() {
        for (name, p) in [("hell", LIGHT), ("dunkel", DARK)] {
            for (fg, bg, min, what) in [
                (p.text, p.base, 4.5, "Text/Grund"),
                (p.text, p.window, 4.5, "Text/Fenster"),
                (p.text, p.alternate, 4.5, "Text/Wechselzeile"),
                (p.muted, p.base, 4.5, "Nebentext"),
                (p.highlight_text, p.highlight, 4.5, "Auswahl"),
                (p.link, p.base, 4.5, "Verweis"),
                (p.warning, p.base, 4.5, "Warnung"),
                (p.error, p.base, 4.5, "Fehler"),
                (p.ok, p.base, 4.5, "OK"),
                (p.text, p.button, 4.5, "Schaltfläche"),
            ] {
                let c = contrast(fg, bg);
                assert!(c >= min, "{name} {what}: {c:.2} < {min}");
            }
        }
    }

    #[test]
    fn contrast_formula() {
        assert!((contrast("#000000", "#ffffff") - 21.0).abs() < 0.01);
        assert!((contrast("#777777", "#777777") - 1.0).abs() < 0.01);
    }
}
