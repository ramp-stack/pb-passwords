//! Display-name changes must not create a second AIR identity.

pub fn application_name() -> String {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        static PREPARED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
        PREPARED.get_or_init(|| {
            let home = std::env::var_os("HOME").expect("Application home is unavailable");
            let support = std::path::PathBuf::from(home).join("Library/Application Support");
            preserve_existing_installation(&support)
                .expect("Could not retain the existing password app data after renaming");
        });
    }
    "Peanut Butter Passwords".into()
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn preserve_existing_installation(support: &std::path::Path) -> std::io::Result<()> {
    let old = support.join("Pickle Passwords");
    let new = support.join("Peanut Butter Passwords");
    // Alias the original directory rather than copying a potentially open AIR
    // database. Existing and previous builds continue to use the same identity.
    // Never replace data already created under the new name.
    if old.is_dir() && std::fs::symlink_metadata(&new).is_err() {
        match std::os::unix::fs::symlink("Pickle Passwords", new) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(all(test, any(target_os = "macos", target_os = "ios")))]
mod tests {
    use super::*;

    #[test]
    fn rebrand_reuses_existing_data_without_copying_or_overwriting() {
        let root = std::env::temp_dir().join(format!("peanut-brand-{}", uuid::Uuid::new_v4()));
        let old = root.join("Pickle Passwords");
        let new = root.join("Peanut Butter Passwords");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("example"), "existing").unwrap();
        preserve_existing_installation(&root).unwrap();
        preserve_existing_installation(&root).unwrap();
        assert_eq!(old.canonicalize().unwrap(), new.canonicalize().unwrap());
        std::fs::write(new.join("example"), "updated").unwrap();
        assert_eq!(
            std::fs::read_to_string(old.join("example")).unwrap(),
            "updated"
        );
        std::fs::remove_file(&new).unwrap();
        std::fs::create_dir(&new).unwrap();
        std::fs::write(new.join("example"), "new installation").unwrap();
        preserve_existing_installation(&root).unwrap();
        assert_eq!(
            std::fs::read_to_string(new.join("example")).unwrap(),
            "new installation"
        );
        assert_eq!(
            std::fs::read_to_string(old.join("example")).unwrap(),
            "updated"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
