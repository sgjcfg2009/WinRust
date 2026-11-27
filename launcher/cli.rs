use crate::*;

pub(crate) fn cli(args: &[String]) -> i32 {
    let stdout_h = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    let redirected = !stdout_h.is_null() && stdout_h != INVALID_HANDLE_VALUE;
    let mut out: Box<dyn Write> = if redirected {
        Box::new(std::io::stdout())
    } else {
        unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
        winrust_common::utf8_console();
        fs::OpenOptions::new().write(true).open("CONOUT$").map_or_else(|_| Box::new(std::io::sink()) as Box<dyn Write>, |f| Box::new(f))
    };
    let mut say = |s: &str| {
        let _ = writeln!(out, "{s}");
    };
    let yes = args.iter().any(|a| a == "--yes");
    let words: Vec<&str> = args.iter().map(String::as_str).filter(|&a| a != "--yes").collect();
    let find = |n: &str| PLUGINS.iter().position(|p| p.name == n);
    let usage = "usage: plugin-launcher list | install <plugin>...|all --yes | update <plugin>|all | uninstall <plugin> | rollback --yes";
    let result = match words.as_slice() {
        ["list"] => {
            for p in PLUGINS {
                say(&format!("{:<16}{}", p.name, if p.actions.is_none() { "stub" } else if (p.installed)() { "installed" } else { "not installed" }));
            }
            return 0;
        }
        ["install", names @ ..] if !names.is_empty() => {
            let mut idx = Vec::new();
            for n in names {
                match *n {
                    "all" => idx.extend(ready().filter(|&i| selected_by_default(PLUGINS[i].name, false))),
                    n => match find(n) {
                        Some(i) => idx.push(i),
                        None => return { say(&format!("unknown plugin: {n}")); 2 },
                    },
                }
            }
            idx.sort_unstable();
            idx.dedup();
            if !yes {
                for &i in &idx {
                    say(&format!("{}: {}", PLUGINS[i].name, PLUGINS[i].warn));
                }
                return { say("Nothing changed. Repeat with --yes to install."); 2 };
            }
            let mut failed = false;
            for i in idx {
                if (PLUGINS[i].installed)() {
                    say(&format!("{} is already installed", PLUGINS[i].name));
                    continue;
                }
                match install_plugin(i) {
                    Ok(m) => say(&m),
                    Err(e) => {
                        failed = true;
                        say(&format!("error: {}: {}", PLUGINS[i].name, e));
                    }
                }
            }
            return if failed { 1 } else { 0 };
        }
        ["update", "all"] => {
            let mut failed = false;
            for i in (0..PLUGINS.len()).filter(|&i| PLUGINS[i].actions.is_some() && (PLUGINS[i].installed)()) {
                match install_plugin(i) {
                    Ok(m) => say(&m),
                    Err(e) => {
                        failed = true;
                        say(&format!("error: {}: {}", PLUGINS[i].name, e));
                    }
                }
            }
            return if failed { 1 } else { 0 };
        }
        ["update", n] => match find(n) {
            None => return { say(&format!("unknown plugin: {n}")); 2 },
            Some(i) if !(PLUGINS[i].installed)() => return { say(&format!("{n} is not installed (use install)")); 2 },
            Some(i) => install_plugin(i),
        },
        ["uninstall", n] => match find(n) {
            None => return { say(&format!("unknown plugin: {n}")); 2 },
            Some(i) if !(PLUGINS[i].installed)() => return { say(&format!("{n} is not installed")); 0 },
            Some(i) => uninstall_plugin(i),
        },
        ["rollback"] if yes => rollback(),
        ["rollback"] => return { say("Removes every installed plugin. Repeat with --yes."); 2 },
        _ => return { say(usage); 2 },
    };
    match result {
        Ok(m) => { say(&m); 0 }
        Err(e) => { say(&format!("error: {e}")); 1 }
    }
}
