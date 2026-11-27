use crate::*;
pub(crate) use i18n::journal::{Entry, svc_mode};

pub(crate) struct Journal(PathBuf);

pub(crate) fn svc_key(name: &str) -> String {
    format!(r"HKLM\SYSTEM\CurrentControlSet\Services\{name}")
}

pub(crate) fn reg_get(key: &str, name: &str) -> Option<(String, String)> {
    let o = out("reg", &["query", key, "/v", name]).ok()?;
    let (ty, d) = i18n::reg::value(&o)?;
    let d = if ty == "REG_DWORD" { i18n::reg::dword(&o)?.to_string() } else { d.to_string() };
    Some((ty.to_string(), d))
}

impl Journal {
    pub(crate) fn open(plugin: &str, legacy: impl FnOnce() -> Vec<Entry>) -> Result<Journal, String> {
        let j = Journal(program_files().join(r"WinRust\journal").join(format!("{plugin}.txt")));
        match fs::metadata(&j.0) {
            Ok(_) => {
                j.entries()?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir_all(j.0.parent().unwrap()).map_err(|e| e.to_string())?;
                let text: String = legacy().iter().filter_map(Entry::line).map(|l| l + "\n").collect();
                let mut f = fs::OpenOptions::new().write(true).create_new(true).open(&j.0)
                    .map_err(|e| format!("{}: {e}", j.0.display()))?;
                f.write_all(text.as_bytes()).and_then(|_| f.sync_all())
                    .map_err(|e| format!("{}: {e}", j.0.display()))?;
            }
            Err(e) => return Err(format!("{}: {e}", j.0.display())),
        }
        Ok(j)
    }

    fn entries(&self) -> Result<Vec<Entry>, String> {
        let path = self.0.display();
        let text = fs::read_to_string(&self.0).map_err(|e| format!("{path}: {e}"))?;
        i18n::journal::parse_all(&text)
            .map_err(|line| format!("{path}: invalid journal record on line {line}"))
    }

    fn record(&self, e: Entry) -> Result<(), String> {
        if self.entries()?.iter().any(|x| x.same(&e)) {
            return Ok(());
        }
        let line = e.line().ok_or_else(|| format!("cannot save the old value: {e:?}"))?;
        let mut f = fs::OpenOptions::new().append(true).open(&self.0).map_err(|e| format!("{}: {e}", self.0.display()))?;
        writeln!(f, "{line}").and_then(|_| f.sync_all()).map_err(|e| format!("{}: {e}", self.0.display()))
    }

    pub(crate) fn set_reg(&self, key: &str, name: &str, ty: &str, data: &str) -> Result<(), String> {
        self.record(Entry::Reg { key: key.into(), name: name.into(), old: reg_get(key, name) })?;
        run("reg", &["add", key, "/v", name, "/t", ty, "/d", data, "/f"])
    }

    pub(crate) fn set_svc(&self, name: &str, mode: &str) -> Result<(), String> {
        let k = svc_key(name);
        let old = svc_mode(reg_dword(&k, "Start"), reg_dword(&k, "DelayedAutostart")).ok_or_else(|| format!("service {name}: unknown start type"))?;
        self.record(Entry::Svc { name: name.into(), mode: old.into() })?;
        run("sc.exe", &["config", name, "start=", mode])
    }

    pub(crate) fn undo(&self) -> Result<(), String> {
        let errs: Vec<String> = self.entries()?.iter().rev().filter_map(|e| restore(e).err()).collect();
        if !errs.is_empty() {
            return Err(errs.join("; "));
        }
        let _ = fs::remove_file(&self.0);
        let _ = fs::remove_dir(self.0.parent().unwrap());
        Ok(())
    }
}

fn restore(e: &Entry) -> Result<(), String> {
    match e {
        Entry::Reg { key, name, old: None } => run("reg", &["delete", key, "/v", name, "/f"]).or_else(|e| if reg_get(key, name).is_none() { Ok(()) } else { Err(e) }),
        Entry::Reg { key, name, old: Some((ty, d)) } => run("reg", &["add", key, "/v", name, "/t", ty, "/d", d, "/f"]),
        Entry::Svc { name, mode } => run("sc.exe", &["config", name, "start=", mode]).or_else(|e| if run("sc.exe", &["query", name]).is_err() { Ok(()) } else { Err(e) }),
    }
}
