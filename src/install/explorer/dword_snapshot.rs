pub(crate) type Dword<'a> = (&'a str, &'a str);

pub(crate) fn backup_and_set<'a>(
    setting: Dword<'a>,
    backup: Dword<'a>,
    new: u32,
    read: impl Fn(Dword<'a>) -> Result<Option<u32>, String>,
    write: impl Fn(Dword<'a>, u32) -> Result<(), String>,
) -> Result<bool, String> {
    let Some(old) = read(setting)? else { return Ok(false) };
    if old == new {
        return Ok(false);
    }
    if read(backup)?.is_none() {
        write(backup, old)?;
    }
    write(setting, new)?;
    Ok(true)
}

pub(crate) fn restore<'a>(
    setting: Dword<'a>,
    backup: Dword<'a>,
    read: impl Fn(Dword<'a>) -> Result<Option<u32>, String>,
    write: impl Fn(Dword<'a>, u32) -> Result<(), String>,
) -> Result<bool, String> {
    let Some(old) = read(backup)? else { return Ok(false) };
    if read(setting)? != Some(old) {
        write(setting, old)?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::HashMap};

    const START: Dword<'static> = ("Services\\Example", "Start");
    const BACKUP: Dword<'static> = ("Installer", "Example");

    #[derive(Default)]
    struct Registry {
        values: RefCell<HashMap<Dword<'static>, u32>>,
        write_error: Option<Dword<'static>>,
    }

    impl Registry {
        fn get(&self, key: Dword<'static>) -> Option<u32> {
            self.values.borrow().get(&key).copied()
        }

        fn write(&self, key: Dword<'static>, value: u32) -> Result<(), String> {
            if self.write_error == Some(key) {
                return Err("access denied".into());
            }
            self.values.borrow_mut().insert(key, value);
            Ok(())
        }

        fn apply(&self, new: u32) -> Result<bool, String> {
            backup_and_set(START, BACKUP, new, |key| Ok(self.get(key)), |key, value| self.write(key, value))
        }

        fn undo(&self) -> Result<bool, String> {
            restore(START, BACKUP, |key| Ok(self.get(key)), |key, value| self.write(key, value))
        }
    }

    #[test]
    fn keeps_the_original_across_retries_and_restores_it() {
        let reg = Registry::default();
        reg.write(START, 3).unwrap();
        assert!(reg.apply(4).unwrap());
        assert!(reg.apply(2).unwrap());
        assert_eq!(reg.get(BACKUP), Some(3));
        assert!(reg.undo().unwrap());
        assert_eq!(reg.get(START), Some(3));
    }

    #[test]
    fn leaves_missing_targets_and_legacy_installations_alone() {
        let reg = Registry::default();
        assert!(!reg.apply(4).unwrap());
        assert!(reg.values.borrow().is_empty());
        reg.write(START, 4).unwrap();
        assert!(!reg.undo().unwrap());
        assert_eq!(reg.get(START), Some(4));
    }

    #[test]
    fn a_failed_snapshot_prevents_the_system_write() {
        let reg = Registry { write_error: Some(BACKUP), ..Registry::default() };
        reg.write(START, 3).unwrap();
        assert!(reg.apply(4).is_err());
        assert_eq!(reg.get(START), Some(3));
    }

    #[test]
    fn a_failed_restore_leaves_the_backup_for_retry() {
        let reg = Registry { write_error: Some(START), ..Registry::default() };
        reg.write(BACKUP, 3).unwrap();
        reg.values.borrow_mut().insert(START, 4);
        assert!(reg.undo().is_err());
        assert_eq!(reg.get(BACKUP), Some(3));
    }

    #[test]
    fn a_read_error_is_not_treated_as_a_missing_value() {
        let reg = Registry::default();
        let result = backup_and_set(START, BACKUP, 4, |_| Err("access denied".into()), |key, value| reg.write(key, value));
        assert!(result.is_err());
        assert!(reg.values.borrow().is_empty());
    }
}
