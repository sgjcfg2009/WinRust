use std::{fs, path::PathBuf};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum RuleAct {
    Lower,
    Notify,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Rule {
    pub proc: String,
    pub cpu: f32,
    pub secs: u32,
    pub act: RuleAct,
    pub hits: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub view: usize,
    pub sort: usize,
    pub desc: bool,
    pub pos: (i32, i32),
    pub size: (u32, u32),
    pub to_tray: bool,
    pub theme: u8,
    pub etw: bool,
    pub gpu_temp_always: bool,
    pub rules: Vec<Rule>,
}

impl Default for Settings {
    fn default() -> Self {
        Self { view: 0, sort: 2, desc: true, pos: (150, 60), size: (1000, 740), to_tray: true, theme: 2, etw: false, gpu_temp_always: false, rules: vec![] }
    }
}

pub fn dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("rtaskmgr")
}

pub fn render(s: &Settings) -> String {
    let mut t = format!(
        "view={}\npsort={}\ndesc={}\nx={}\ny={}\nw={}\nh={}\nto_tray={}\ntheme={}\netw={}\ngputemp={}\n",
        s.view, s.sort, s.desc as u8, s.pos.0, s.pos.1, s.size.0, s.size.1, s.to_tray as u8, s.theme, s.etw as u8, s.gpu_temp_always as u8
    );
    for r in &s.rules {
        t += &format!("rule={}|{}|{}|{}|{}\n", r.proc, r.cpu, r.secs, if r.act == RuleAct::Lower { "lower" } else { "notify" }, r.hits);
    }
    t
}

pub fn parse(text: &str) -> Settings {
    let mut s = Settings::default();
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        match k {
            "view" => s.view = v.parse().unwrap_or(s.view),
            "psort" => s.sort = v.parse().unwrap_or(s.sort),
            "desc" => s.desc = v == "1",
            "x" => s.pos.0 = v.parse().unwrap_or(s.pos.0),
            "y" => s.pos.1 = v.parse().unwrap_or(s.pos.1),
            "w" => s.size.0 = v.parse().unwrap_or(s.size.0),
            "h" => s.size.1 = v.parse().unwrap_or(s.size.1),
            "to_tray" => s.to_tray = v == "1",
            "theme" => s.theme = v.parse().unwrap_or(2).min(2),
            "etw" => s.etw = v == "1",
            "gputemp" => s.gpu_temp_always = v == "1",
            "rule" => {
                let p: Vec<&str> = v.split('|').collect();
                if let [name, cpu, secs, act, hits] = p[..]
                    && let (Ok(cpu), Ok(secs)) = (cpu.parse(), secs.parse()) {
                        s.rules.push(Rule {
                            proc: name.to_string(),
                            cpu,
                            secs,
                            act: if act == "lower" { RuleAct::Lower } else { RuleAct::Notify },
                            hits: hits.parse().unwrap_or(0),
                        });
                    }
            }
            _ => {}
        }
    }
    s.size = (s.size.0.clamp(640, 4000), s.size.1.clamp(480, 3000));
    s.pos = (s.pos.0.clamp(-200, 6000), s.pos.1.clamp(-50, 4000));
    s
}

pub fn load() -> Settings {
    fs::read_to_string(dir().join("settings.ini")).map(|t| parse(&t)).unwrap_or_default()
}

pub fn save(s: &Settings) {
    let _ = fs::create_dir_all(dir());
    let _ = fs::write(dir().join("settings.ini"), render(s));
}

fn hist_path() -> PathBuf {
    dir().join("history.csv")
}

pub fn trim_history(max: usize) {
    let Ok(t) = fs::read_to_string(hist_path()) else { return };
    let lines: Vec<&str> = t.lines().collect();
    if lines.len() > max {
        let _ = fs::write(hist_path(), lines[lines.len() - max..].join("\n") + "\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_roundtrip() {
        let mut s = Settings::default();
        s.view = 3;
        s.sort = 5;
        s.theme = 1;
        s.etw = true;
        s.gpu_temp_always = true;
        s.desc = false;
        s.rules.push(Rule { proc: "chrome.exe".into(), cpu: 50.0, secs: 60, act: RuleAct::Lower, hits: 2 });
        assert_eq!(parse(&render(&s)), s);
    }

    #[test]
    fn garbage_is_clamped_and_ignored() {
        let s = parse("w=1\nh=99999\nx=-99999\nrule=bad\nnonsense\nview=abc");
        assert_eq!(s.size, (640, 3000));
        assert_eq!(s.pos.0, -200);
        assert!(s.rules.is_empty());
        assert_eq!(s.view, 0);
    }
}
