//! Versionen nach SemVer 2.0 (Vorrang laut Abschnitt 11, Build-Angaben ohne Einfluss).

use std::cmp::Ordering;
use std::fmt;

use crate::UpdateError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// Vorabkennung, z. B. `["rc", "1"]`; leer bei einer Freigabe.
    pub pre: Vec<String>,
}

fn number(s: &str, all: &str) -> Result<u64, UpdateError> {
    if s.is_empty() || (s.len() > 1 && s.starts_with('0')) || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(UpdateError::Version(all.into()));
    }
    s.parse().map_err(|_| UpdateError::Version(all.into()))
}

impl Version {
    /// Liest `1.2.3`, `v1.2.3`, `1.2.3-rc.1+build.5`.
    pub fn parse(s: &str) -> Result<Version, UpdateError> {
        let t = s.trim();
        let t = t.strip_prefix('v').or_else(|| t.strip_prefix('V')).unwrap_or(t);
        let t = t.split('+').next().unwrap_or("");
        let (core, pre) = match t.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (t, None),
        };
        let parts: Vec<&str> = core.split('.').collect();
        if parts.len() != 3 {
            return Err(UpdateError::Version(s.into()));
        }
        let pre = match pre {
            None => Vec::new(),
            Some(p) => {
                let ids: Vec<String> = p.split('.').map(str::to_string).collect();
                for id in &ids {
                    let ok = !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
                    let numeric = id.bytes().all(|b| b.is_ascii_digit());
                    if !ok || (numeric && id.len() > 1 && id.starts_with('0')) {
                        return Err(UpdateError::Version(s.into()));
                    }
                }
                ids
            }
        };
        Ok(Version { major: number(parts[0], s)?, minor: number(parts[1], s)?, patch: number(parts[2], s)?, pre })
    }

    pub fn is_prerelease(&self) -> bool {
        !self.pre.is_empty()
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if !self.pre.is_empty() {
            write!(f, "-{}", self.pre.join("."))?;
        }
        Ok(())
    }
}

fn cmp_ident(a: &str, b: &str) -> Ordering {
    match (a.parse::<u64>(), b.parse::<u64>()) {
        (Ok(x), Ok(y)) if a.bytes().all(|c| c.is_ascii_digit()) && b.bytes().all(|c| c.is_ascii_digit()) => x.cmp(&y),
        // Zahlen stehen vor Bezeichnern.
        (Ok(_), Err(_)) if a.bytes().all(|c| c.is_ascii_digit()) => Ordering::Less,
        (Err(_), Ok(_)) if b.bytes().all(|c| c.is_ascii_digit()) => Ordering::Greater,
        _ => a.cmp(b),
    }
}

impl Ord for Version {
    fn cmp(&self, o: &Self) -> Ordering {
        (self.major, self.minor, self.patch).cmp(&(o.major, o.minor, o.patch)).then_with(|| match (self.pre.is_empty(), o.pre.is_empty()) {
            (true, true) => Ordering::Equal,
            // Eine Vorabversion steht vor der Freigabe.
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => {
                for (a, b) in self.pre.iter().zip(&o.pre) {
                    let c = cmp_ident(a, b);
                    if c != Ordering::Equal {
                        return c;
                    }
                }
                self.pre.len().cmp(&o.pre.len())
            }
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn semver_precedence() {
        // Reihenfolge aus SemVer 2.0, Abschnitt 11.
        let order = ["1.0.0-alpha", "1.0.0-alpha.1", "1.0.0-alpha.beta", "1.0.0-beta", "1.0.0-beta.2", "1.0.0-beta.11", "1.0.0-rc.1", "1.0.0", "1.0.1", "1.1.0", "2.0.0"];
        for w in order.windows(2) {
            assert!(v(w[0]) < v(w[1]), "{} < {}", w[0], w[1]);
        }
        assert_eq!(v("v1.2.3+build.7"), v("1.2.3"));
        assert_eq!(v("0.10.0").cmp(&v("0.9.9")), Ordering::Greater);
        assert!(v("1.0.0-rc.1").is_prerelease());
        assert_eq!(v("1.0.0-rc.1").to_string(), "1.0.0-rc.1");
        for bad in ["1.2", "1.2.3.4", "01.2.3", "1.2.3-", "1.2.3-01", "a.b.c", "", "1.2.3-ä"] {
            assert!(Version::parse(bad).is_err(), "{bad}");
        }
    }
}
