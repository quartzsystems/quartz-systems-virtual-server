// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Service tools user IDs of an appliance: who may sign on at its
//! local console. The users, their Argon2id password hashes and the
//! file they are kept in are QS/400's `svcusers`; what is the
//! appliance's own is here: the password rules, every user able to do
//! everything (there is one kind of user in this release), and the
//! sign-on rule: five wrong passwords disable an ID for five minutes.
//!
//! No password is kept, logged or returned anywhere: only its hash.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::PathBuf;

use svcusers::{FileStore, Privilege, ServiceUser, UserDb, UserStore, verify_password};

/// Wrong passwords before an ID is disabled for a while.
pub const MAX_FAILURES: u8 = 5;
/// Seconds an ID stays disabled.
pub const LOCK_SECS: u64 = 300;
/// The shortest password.
pub const PASSWORD_MIN: usize = 8;
/// The longest password.
pub const PASSWORD_MAX: usize = svcusers::PASSWORD_LEN;
/// The longest user ID.
pub const ID_MAX: usize = svcusers::NAME_LEN;
/// The users' file in a state directory.
pub const FILE: &str = "users.db";

/// The rules, as a screen states them.
pub const RULES: [&str; 4] = [
    "A user ID is 1 to 10 letters and digits. It is kept in upper case.",
    "A password is 8 to 128 characters and is case sensitive.",
    "It has at least one letter and at least one digit.",
    "It does not contain the user ID.",
];

/// Why a sign-on was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// No such user, or the password does not match. Both read the same.
    NotCorrect,
    /// The ID is disabled for this many more seconds.
    Disabled(u64),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::NotCorrect => f.write_str("User ID or password not correct."),
            Refused::Disabled(seconds) => write!(
                f,
                "User ID disabled after {MAX_FAILURES} wrong passwords. Try again in {} min.",
                seconds.div_ceil(60)
            ),
        }
    }
}

/// A user ID as kept, or why it is not one.
pub fn check_id(id: &str) -> Result<String, String> {
    let id = svcusers::normalize_name(id);
    if id.is_empty() || id.len() > ID_MAX || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err("User ID must be 1 to 10 letters and digits.".into());
    }
    Ok(id)
}

/// Whether `password` keeps the rules for the user `id`.
pub fn check_password(id: &str, password: &str) -> Result<(), String> {
    let count = password.chars().count();
    if count < PASSWORD_MIN {
        return Err(format!(
            "Password must be at least {PASSWORD_MIN} characters."
        ));
    }
    if password.len() > PASSWORD_MAX {
        return Err(format!(
            "Password must be at most {PASSWORD_MAX} characters."
        ));
    }
    if password.chars().any(char::is_control) {
        return Err("Password must not have control characters.".into());
    }
    if !password.chars().any(char::is_alphabetic) || !password.chars().any(|c| c.is_ascii_digit()) {
        return Err("Password must have a letter and a digit.".into());
    }
    let id = svcusers::normalize_name(id);
    if !id.is_empty() && password.to_ascii_uppercase().contains(&id) {
        return Err("Password must not contain the user ID.".into());
    }
    Ok(())
}

/// Sixteen random bytes from the kernel.
fn salt() -> io::Result<[u8; 16]> {
    let mut bytes = [0u8; 16];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// The hash of `password` under a new salt, in PHC string form.
pub fn hash(password: &str) -> io::Result<String> {
    Ok(svcusers::hash_password(password, salt()?))
}

/// Whether `text` has the form of an Argon2id hash this code verifies.
pub fn valid_hash(text: &str) -> bool {
    let parts: Vec<&str> = text.split('$').collect();
    parts.len() == 6
        && parts[0].is_empty()
        && parts[1] == "argon2id"
        && parts[2].starts_with("v=")
        && parts[3].contains("m=")
        && parts[4..].iter().all(|p| {
            !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
        })
}

fn user(id: String, hash: String, now: u64) -> ServiceUser {
    ServiceUser {
        name: id,
        privileges: Privilege::ALL,
        hash,
        expired: false,
        disabled: false,
        failures: 0,
        changed: now,
        text: String::new(),
    }
}

/// The bytes of a users' file holding one user: what an installer
/// writes.
pub fn first(id: &str, hash: &str, now: u64) -> Result<Vec<u8>, String> {
    let id = check_id(id)?;
    if !valid_hash(hash) {
        return Err("The password hash is not an Argon2id hash.".into());
    }
    let mut db = UserDb::default();
    db.push(user(id, hash.to_string(), now));
    Ok(db.encode())
}

/// The users of an appliance, in their store.
pub struct Users<S> {
    db: UserDb,
    store: S,
    /// IDs disabled, and until when (seconds since the epoch).
    disabled: HashMap<String, u64>,
}

impl Users<FileStore> {
    /// The users in the file at `path`; none when it is not there.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, String> {
        Self::load(FileStore::new(path))
    }
}

impl<S: UserStore> Users<S>
where
    S::Error: std::fmt::Display,
{
    /// The users `store` holds.
    pub fn load(mut store: S) -> Result<Self, String> {
        let db = UserDb::read_from(&mut store)
            .map_err(|e| format!("the users were not read: {e}"))?
            .unwrap_or_default();
        Ok(Self {
            db,
            store,
            disabled: HashMap::new(),
        })
    }

    fn keep(&mut self) -> Result<(), String> {
        self.db
            .write_to(&mut self.store)
            .map_err(|e| format!("The users were not written: {e}"))
    }

    /// The user IDs, in the order they were made.
    pub fn ids(&self) -> Vec<String> {
        self.db.list().iter().map(|u| u.name.clone()).collect()
    }

    /// How many seconds `id` is still disabled for, at `now`.
    pub fn disabled_for(&self, id: &str, now: u64) -> Option<u64> {
        let until = *self.disabled.get(&svcusers::normalize_name(id))?;
        (until > now).then(|| until - now)
    }

    /// Check a user ID and password at `now`: the ID as kept. The
    /// fifth wrong password in a row disables the ID for a while.
    pub fn sign_on(&mut self, id: &str, password: &str, now: u64) -> Result<String, Refused> {
        let id = svcusers::normalize_name(id);
        if let Some(left) = self.disabled_for(&id, now) {
            return Err(Refused::Disabled(left));
        }
        let Some(user) = self.db.get_mut(&id) else {
            return Err(Refused::NotCorrect);
        };
        if verify_password(&user.hash, password) {
            if user.failures != 0 {
                user.failures = 0;
                let _ = self.keep();
            }
            return Ok(id);
        }
        user.failures += 1;
        let refused = if user.failures >= MAX_FAILURES {
            user.failures = 0;
            self.disabled.insert(id, now + LOCK_SECS);
            Refused::Disabled(LOCK_SECS)
        } else {
            Refused::NotCorrect
        };
        let _ = self.keep();
        Err(refused)
    }

    /// Add a user.
    pub fn add(&mut self, id: &str, password: &str, now: u64) -> Result<(), String> {
        let id = check_id(id)?;
        check_password(&id, password)?;
        if self.db.get(&id).is_some() {
            return Err(format!("User ID {id} exists already."));
        }
        let hash = hash(password).map_err(|e| format!("No random bytes: {e}"))?;
        self.db.push(user(id, hash, now));
        self.keep()
    }

    /// Give a user a new password, and enable the ID.
    pub fn set_password(&mut self, id: &str, password: &str, now: u64) -> Result<(), String> {
        let id = check_id(id)?;
        check_password(&id, password)?;
        let hash = hash(password).map_err(|e| format!("No random bytes: {e}"))?;
        let Some(user) = self.db.get_mut(&id) else {
            return Err(format!("There is no user ID {id}."));
        };
        user.hash = hash;
        user.failures = 0;
        user.changed = now;
        self.disabled.remove(&id);
        self.keep()
    }

    /// Remove a user. The last one stays.
    pub fn remove(&mut self, id: &str) -> Result<(), String> {
        let id = check_id(id)?;
        if self.db.get(&id).is_none() {
            return Err(format!("There is no user ID {id}."));
        }
        if self.db.list().len() == 1 {
            return Err("The last user ID cannot be removed.".into());
        }
        self.db.delete(&id).map_err(|e| e.to_string())?;
        self.disabled.remove(&id);
        self.keep()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A store in memory.
    #[derive(Default)]
    struct Mem(Option<Vec<u8>>);

    impl UserStore for &mut Mem {
        type Error = String;
        fn read(&mut self) -> Result<Option<Vec<u8>>, String> {
            Ok(self.0.clone())
        }
        fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
            self.0 = Some(bytes.to_vec());
            Ok(())
        }
    }

    const PASSWORD: &str = "correct horse 7";

    #[test]
    fn the_rules_are_kept() {
        assert_eq!(check_id(" admin "), Ok("ADMIN".into()));
        assert!(check_id("").is_err());
        assert!(check_id("elevenchars").is_err());
        assert!(check_id("a b").is_err());
        assert_eq!(check_password("ADMIN", PASSWORD), Ok(()));
        for bad in [
            "short1",
            "nodigitshere",
            "12345678",
            "xadmin123",
            "tab\there 1",
        ] {
            assert!(check_password("ADMIN", bad).is_err(), "{bad}");
        }
        assert!(check_password("ADMIN", &"a1".repeat(65)).is_err());
    }

    #[test]
    fn five_wrong_passwords_disable_an_id_for_five_minutes() {
        let mut mem = Mem::default();
        let mut users = Users::load(&mut mem).unwrap();
        users.add("admin", PASSWORD, 100).unwrap();
        assert_eq!(users.sign_on("Admin", PASSWORD, 100), Ok("ADMIN".into()));
        assert_eq!(
            users.sign_on("nobody", PASSWORD, 100),
            Err(Refused::NotCorrect)
        );
        for _ in 0..4 {
            assert_eq!(
                users.sign_on("admin", "wrong", 100),
                Err(Refused::NotCorrect)
            );
        }
        assert_eq!(
            users.sign_on("admin", "wrong", 100),
            Err(Refused::Disabled(LOCK_SECS))
        );
        // The right password does not help while it lasts.
        assert_eq!(
            users.sign_on("admin", PASSWORD, 160),
            Err(Refused::Disabled(LOCK_SECS - 60))
        );
        assert_eq!(users.disabled_for("admin", 160), Some(240));
        assert_eq!(
            users.sign_on("admin", PASSWORD, 100 + LOCK_SECS),
            Ok("ADMIN".into())
        );
        // A good sign-on starts the count again.
        for _ in 0..4 {
            assert!(users.sign_on("admin", "wrong", 500).is_err());
        }
        assert!(users.sign_on("admin", PASSWORD, 500).is_ok());
        for _ in 0..4 {
            assert_eq!(
                users.sign_on("admin", "wrong", 500),
                Err(Refused::NotCorrect)
            );
        }
    }

    #[test]
    fn users_are_added_changed_and_removed_and_the_last_one_stays() {
        let mut mem = Mem::default();
        let mut users = Users::load(&mut mem).unwrap();
        users.add("admin", PASSWORD, 1).unwrap();
        assert!(
            users
                .add("ADMIN", PASSWORD, 1)
                .unwrap_err()
                .contains("exists")
        );
        assert!(users.remove("admin").unwrap_err().contains("last"));
        users.add("second", "another pass 9", 2).unwrap();
        assert_eq!(users.ids(), ["ADMIN", "SECOND"]);
        users.set_password("second", "changed pass 10", 3).unwrap();
        assert!(users.sign_on("second", "another pass 9", 3).is_err());
        assert!(users.sign_on("second", "changed pass 10", 3).is_ok());
        users.remove("admin").unwrap();
        assert!(users.remove("second").unwrap_err().contains("last"));
        assert!(users.remove("ghost").is_err());

        // What is kept has the hash and never the password.
        let kept = mem.0.clone().unwrap();
        let text = String::from_utf8_lossy(&kept);
        assert!(text.contains("$argon2id$"));
        assert!(!text.contains("changed pass 10"));
        let again = Users::load(&mut mem).unwrap();
        assert_eq!(again.ids(), ["SECOND"]);
    }

    #[test]
    fn an_installer_s_file_holds_the_one_user_it_was_given() {
        let hash = svcusers::hash_password(PASSWORD, [7; 16]);
        assert!(valid_hash(&hash));
        assert!(!valid_hash("$argon2i$v=19$m=1$abc$def"));
        assert!(!valid_hash(PASSWORD));
        let mut mem = Mem(Some(first("admin", &hash, 9).unwrap()));
        let mut users = Users::load(&mut mem).unwrap();
        assert_eq!(users.ids(), ["ADMIN"]);
        assert!(users.sign_on("admin", PASSWORD, 9).is_ok());
        assert!(first("admin", "plain", 9).is_err());
        assert!(first("", &hash, 9).is_err());
    }
}
