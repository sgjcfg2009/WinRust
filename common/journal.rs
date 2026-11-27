#[derive(Debug, PartialEq, Clone)]
pub enum Entry {
    Reg { key: String, name: String, old: Option<(String, String)> },
    Svc { name: String, mode: String },
}

const TYPES: [&str; 3] = ["REG_SZ", "REG_EXPAND_SZ", "REG_DWORD"];

impl Entry {
    pub fn line(&self) -> Option<String> {
        let ok = |s: &str| !s.contains(['\r', '\n']);
        match self {
            Entry::Reg { key, name, old } => {
                if name.is_empty() || !ok(key) || !ok(name) || key.contains('|') || name.contains('|') {
                    return None;
                }
                match old {
                    None => Some(format!("reg|{key}|{name}|-")),
                    Some((ty, d)) if TYPES.contains(&ty.as_str()) && ok(d) => Some(format!("reg|{key}|{name}|{ty}|{d}")),
                    Some(_) => None,
                }
            }
            Entry::Svc { name, mode } => (ok(name) && !name.contains('|') && ["auto", "delayed-auto", "demand", "disabled"].contains(&mode.as_str())).then(|| format!("svc|{name}|{mode}")),
        }
    }

    pub fn parse(l: &str) -> Option<Entry> {
        let mut parts = [""; 5];
        let mut n = 0;
        for part in l.trim_end_matches(['\r', '\n']).splitn(5, '|') {
            parts[n] = part;
            n += 1;
        }
        match &parts[..n] {
            ["reg", k, name, "-"] if !name.is_empty() => Some(Entry::Reg { key: (*k).into(), name: (*name).into(), old: None }),
            ["reg", k, name, ty, val] if !name.is_empty() && TYPES.contains(ty) => Some(Entry::Reg { key: (*k).into(), name: (*name).into(), old: Some(((*ty).into(), (*val).into())) }),
            ["svc", name, mode] => Some(Entry::Svc { name: (*name).into(), mode: (*mode).into() }).filter(|e| e.line().is_some()),
            _ => None,
        }
    }

    pub fn same(&self, o: &Entry) -> bool {
        match (self, o) {
            (Entry::Reg { key: a, name: b, .. }, Entry::Reg { key: c, name: d, .. }) => a.eq_ignore_ascii_case(c) && b.eq_ignore_ascii_case(d),
            (Entry::Svc { name: a, .. }, Entry::Svc { name: b, .. }) => a.eq_ignore_ascii_case(b),
            _ => false,
        }
    }
}

pub fn parse_all(text: &str) -> Result<Vec<Entry>, usize> {
    text.split_inclusive('\n')
        .enumerate()
        .map(|(i, line)| line.strip_suffix('\n').and_then(Entry::parse).ok_or(i + 1))
        .collect()
}

pub fn svc_mode(start: Option<u32>, delayed: Option<u32>) -> Option<&'static str> {
    match (start?, delayed.unwrap_or(0)) {
        (2, 0) => Some("auto"),
        (2, _) => Some("delayed-auto"),
        (3, _) => Some("demand"),
        (4, _) => Some("disabled"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(old: Option<(&str, &str)>) -> Entry {
        Entry::Reg { key: r"HKLM\SOFTWARE\X".into(), name: "Debugger".into(), old: old.map(|(t, d)| (t.into(), d.into())) }
    }

    #[test]
    fn lines_round_trip() {
        for e in [reg(None), reg(Some(("REG_SZ", r#""C:\a|b.exe" /x"#))), reg(Some(("REG_DWORD", "26"))), Entry::Svc { name: "WSearch".into(), mode: "delayed-auto".into() }] {
            assert_eq!(Entry::parse(&e.line().unwrap()), Some(e));
        }
    }

    #[test]
    fn reads_empty_journal_and_crlf_records_in_order() {
        assert_eq!(parse_all(""), Ok(vec![]));
        assert_eq!(parse_all("reg|K|V|REG_SZ|a|b\r\nsvc|S|auto\r\n").unwrap(), vec![
            Entry::Reg { key: "K".into(), name: "V".into(), old: Some(("REG_SZ".into(), "a|b".into())) },
            Entry::Svc { name: "S".into(), mode: "auto".into() },
        ]);
    }

    #[test]
    fn rejects_the_entire_journal_when_a_record_is_invalid_or_incomplete() {
        assert_eq!(parse_all("reg|K|V|-\r\ngarbage\nsvc|S|auto\n"), Err(2));
        assert_eq!(parse_all("svc|S|boot\n"), Err(1));
        assert_eq!(parse_all("svc|S|auto\nreg|K|V|REG_SZ|partial"), Err(2));
    }

    #[test]
    fn refuses_what_it_cannot_restore() {
        assert_eq!(reg(Some(("REG_BINARY", "00ff"))).line(), None);
        assert_eq!(reg(Some(("REG_SZ", "a\nb"))).line(), None);
        assert_eq!(Entry::Reg { key: "K".into(), name: "".into(), old: None }.line(), None);
        assert_eq!(Entry::Svc { name: "S".into(), mode: "boot".into() }.line(), None);
        assert_eq!(Entry::parse("svc|S|boot"), None);
        assert_eq!(Entry::parse("reg|K|V|REG_BINARY|00"), None);
    }

    #[test]
    fn same_value_any_case() {
        let other = Entry::Reg { key: r"hklm\software\x".into(), name: "DEBUGGER".into(), old: Some(("REG_SZ".into(), "y".into())) };
        assert!(reg(None).same(&other));
        assert!(!reg(None).same(&Entry::Svc { name: "Debugger".into(), mode: "auto".into() }));
    }

    #[test]
    fn service_modes() {
        assert_eq!(svc_mode(Some(2), None), Some("auto"));
        assert_eq!(svc_mode(Some(2), Some(1)), Some("delayed-auto"));
        assert_eq!(svc_mode(Some(4), Some(1)), Some("disabled"));
        assert_eq!(svc_mode(Some(1), None), None);
        assert_eq!(svc_mode(None, None), None);
    }
}
