//! What registration writes, as data rather than as calls.
//!
//! # Why the footprint is a value
//!
//! `--unregister` has to "remove everything it added and nothing else", and the only way to be sure
//! of that is for both directions to read the same list. So [`plan`] returns the whole footprint and
//! the two operations are folds over it: registering writes each step, unregistering removes each
//! step in reverse. A key that registration adds and unregistration forgets cannot exist, because
//! there is one list.
//!
//! It also makes the part that matters testable without touching a registry. What the tests here
//! assert is not "the call succeeded" — it is **that nothing in the footprint leaves `HKEY_CURRENT_USER`**,
//! which is the security property `AGENTS.md` states as "per-user install, never elevate".
//!
//! # Owned and shared keys are different, and unregister is why
//!
//! The `ProgID` tree and the CLSID are ours: nothing else writes there, so removing them means
//! deleting the subtree. `.md` is **not** ours — it exists whether or not this editor does, and other
//! applications write to it. Deleting it to undo our registration would take somebody else's
//! association with it. So a [`Step::Shared`] entry names one value inside a key belonging to the
//! system, and unregistering removes that value alone.

use std::path::Path;

/// The COM class that Explorer activates to hand over a selection.
///
/// A fixed GUID because it has to be: it is written into the registry by one build and activated by
/// Explorer against another. Generating one per install would leave a dead CLSID behind on every
/// upgrade.
pub const CLSID: &str = "{9F2B4C6D-1E3A-4F58-9B7C-2D5E8A0F1C34}";

/// The `ProgID` the file types point at.
pub const PROGID: &str = "SimpleMarkdownEditor.Document";

/// What the `ProgID` is called where the shell shows a type name.
pub const TYPE_NAME: &str = "Markdown Document";

/// The extensions claimed.
///
/// **`.json`, `.yaml` and `.yml` are deliberately absent.** The editor highlights them, which is not
/// the same as wanting to be the thing that opens every JSON file on the machine — an editor that
/// claims a general-purpose data format uninvited is behaving like something the user would want to
/// remove.
pub const EXTENSIONS: [&str; 2] = [".md", ".markdown"];

/// Where all of this lives. Relative to `HKEY_CURRENT_USER`, and there is no other root.
pub const CLASSES: &str = "Software\\Classes";

/// One registry value the footprint contains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// A key nothing else writes to. Unregistering deletes the key and everything under it.
    ///
    /// `root` is the subtree to remove, which is an ancestor of `key` rather than `key` itself: the
    /// `ProgID` has four keys under it and removing them one at a time would leave the empty shells.
    Owned {
        /// The subtree `--unregister` deletes.
        root: String,
        /// The key this value goes in, under [`CLASSES`].
        key: String,
        /// The value's name, or `None` for the key's default value.
        value: Option<String>,
        /// What to write.
        data: String,
    },
    /// A value inside a key the system owns. Unregistering removes the value and leaves the key.
    Shared {
        /// The key this value goes in, under [`CLASSES`].
        key: String,
        /// The value's name, or `None` for the key's default value.
        value: Option<String>,
        /// What to write.
        data: String,
    },
}

impl Step {
    /// The key this step writes into, under [`CLASSES`].
    #[must_use]
    pub fn key(&self) -> &str {
        match self {
            Self::Owned { key, .. } | Self::Shared { key, .. } => key,
        }
    }

    /// The value name, or `None` for the key's default.
    #[must_use]
    pub fn value(&self) -> Option<&str> {
        match self {
            Self::Owned { value, .. } | Self::Shared { value, .. } => value.as_deref(),
        }
    }

    /// What gets written.
    #[must_use]
    pub fn data(&self) -> &str {
        match self {
            Self::Owned { data, .. } | Self::Shared { data, .. } => data,
        }
    }
}

/// Everything registration writes, in the order it writes it.
///
/// `exe` is the editor's own path, and goes into `LocalServer32` — which is what makes Explorer
/// launch *this* build rather than whichever one registered last, and is why registration is
/// something a person runs rather than something that happens at startup.
#[must_use]
pub fn plan(exe: &Path) -> Vec<Step> {
    let exe = exe.display().to_string();
    let clsid_root = format!("CLSID\\{CLSID}");
    let mut steps = Vec::new();

    // The class Explorer activates. `LocalServer32` rather than `InprocServer32` is the whole
    // ticket: an out-of-process server is one process serving every activation, where an in-process
    // one would be a DLL loaded into Explorer with no editor to open anything in.
    steps.push(Step::Owned {
        root: clsid_root.clone(),
        key: clsid_root.clone(),
        value: None,
        data: "Simple Markdown Editor".to_owned(),
    });
    steps.push(Step::Owned {
        root: clsid_root.clone(),
        key: format!("{clsid_root}\\LocalServer32"),
        value: None,
        data: format!("\"{exe}\""),
    });

    // The type, and the verb that opens it.
    steps.push(Step::Owned {
        root: PROGID.to_owned(),
        key: PROGID.to_owned(),
        value: None,
        data: TYPE_NAME.to_owned(),
    });
    // Index 0 is "the first icon group in the file", not "resource id 0". MDEDIT-70 embedded one, so
    // this now resolves; before it, a registered `.md` file drew the generic document icon.
    steps.push(Step::Owned {
        root: PROGID.to_owned(),
        key: format!("{PROGID}\\DefaultIcon"),
        value: None,
        data: format!("\"{exe}\",0"),
    });
    // **The DropTarget verb, which is the point of the ticket.** A `command` verb defaults to
    // `MultiSelectModel=Document`, which spawns one process per file and silently drops the verb
    // past fifteen — the Notepad++ behaviour `AGENTS.md` names. Explorer prefers `DropTarget` over
    // `command` when both are present, so the command below is a fallback for anything that invokes
    // the verb the old way rather than a second implementation of it.
    steps.push(Step::Owned {
        root: PROGID.to_owned(),
        key: format!("{PROGID}\\shell\\open\\DropTarget"),
        value: Some("CLSID".to_owned()),
        data: CLSID.to_owned(),
    });
    steps.push(Step::Owned {
        root: PROGID.to_owned(),
        key: format!("{PROGID}\\shell\\open\\command"),
        value: None,
        data: format!("\"{exe}\" \"%1\""),
    });

    // And the extensions. `OpenWithProgids` is **additive**: it puts this editor in the shell's
    // Open With list without taking an association away from whatever holds it. Becoming the
    // *default* is deliberately not attempted — see `sets_no_default_association` below.
    for extension in EXTENSIONS {
        steps.push(Step::Shared {
            key: format!("{extension}\\OpenWithProgids"),
            value: Some(PROGID.to_owned()),
            data: String::new(),
        });
    }

    steps
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steps() -> Vec<Step> {
        plan(Path::new("C:\\Program Files\\mdedit\\mdedit.exe"))
    }

    /// **The security property, and the reason this is a test rather than a comment.** `AGENTS.md`
    /// requires a per-user install that never elevates, and per-user registration is what makes that
    /// true by construction: every key here is relative to `Software\Classes` under
    /// `HKEY_CURRENT_USER`, and writing any of them needs no privilege the process does not already
    /// have. A step naming `HKEY_LOCAL_MACHINE`, or reaching up out of `Software\Classes`, would be
    /// asking for a UAC prompt — which is the thing that breaks drag-and-drop from a non-elevated
    /// Explorer.
    #[test]
    fn every_key_is_under_the_user_s_own_classes() {
        for step in steps() {
            let key = step.key();
            assert!(
                !key.contains(".."),
                "{key} climbs out of the classes subtree"
            );
            assert!(
                !key.to_ascii_uppercase().contains("HKEY"),
                "{key} names a hive of its own"
            );
            assert!(!key.starts_with('\\'), "{key} is not relative");
        }
    }

    /// Unregistering deletes subtrees, so an owned root must be an ancestor of the key it covers —
    /// otherwise the delete misses and the key survives the uninstall.
    #[test]
    fn an_owned_root_covers_its_own_key() {
        for step in steps() {
            if let Step::Owned { root, key, .. } = &step {
                assert!(
                    key == root || key.starts_with(&format!("{root}\\")),
                    "{key} is not under {root}"
                );
            }
        }
    }

    /// **The file types are not ours to delete.** `.md` exists whether or not this editor is
    /// installed, and other applications write to it — so every step touching one is `Shared`, which
    /// unregister honours by removing a single value rather than the key.
    #[test]
    fn nothing_claims_ownership_of_a_file_extension() {
        for step in steps() {
            if let Step::Owned { root, .. } = &step {
                assert!(
                    !root.starts_with('.'),
                    "{root} is an extension, and deleting it would take somebody else's association"
                );
            }
        }
    }

    /// **No default association is written, and that is a limitation rather than a decision.**
    /// Windows 8 onwards protects the user's chosen handler with a hash in `UserChoice` that cannot
    /// be produced programmatically, so an editor that writes the `.md` default either has no effect
    /// or is fighting the user. This registers the `ProgID` and offers it; making it the default is
    /// the user's click in Open With → Always.
    #[test]
    fn sets_no_default_association() {
        for step in steps() {
            let key = step.key();
            let claims_default = EXTENSIONS
                .iter()
                .any(|extension| key == *extension && step.value().is_none());
            assert!(!claims_default, "{key} writes the default handler");
        }
    }

    /// The verb has to carry a `DropTarget`, or Explorer falls back to the command line and this
    /// whole ticket has bought nothing.
    #[test]
    fn the_open_verb_carries_a_drop_target() {
        let drop_target = steps().into_iter().find(|step| {
            step.key() == format!("{PROGID}\\shell\\open\\DropTarget")
                && step.value() == Some("CLSID")
        });
        assert_eq!(
            drop_target.map(|step| step.data().to_owned()),
            Some(CLSID.to_owned())
        );
    }

    /// The server is out-of-process. An `InprocServer32` here would be a DLL in Explorer.
    #[test]
    fn the_class_is_a_local_server() {
        let keys: Vec<String> = steps().iter().map(|step| step.key().to_owned()).collect();
        assert!(keys.iter().any(|key| key.ends_with("\\LocalServer32")));
        assert!(!keys.iter().any(|key| key.contains("InprocServer32")));
    }

    /// The executable's own path is what makes Explorer launch this build, so it has to reach both
    /// the class and the fallback verb.
    #[test]
    fn the_executable_path_reaches_the_server_and_the_fallback() {
        let steps = plan(Path::new("C:\\somewhere\\mdedit.exe"));
        let server = steps
            .iter()
            .find(|step| step.key().ends_with("\\LocalServer32"))
            .map(Step::data);
        assert_eq!(server, Some("\"C:\\somewhere\\mdedit.exe\""));

        let command = steps
            .iter()
            .find(|step| step.key().ends_with("\\shell\\open\\command"))
            .map(Step::data);
        assert_eq!(command, Some("\"C:\\somewhere\\mdedit.exe\" \"%1\""));
    }

    /// Registering twice must be the same as registering once — the ticket's own acceptance line.
    /// It holds because the footprint is a pure function of the executable path and every step is a
    /// write of a known value rather than an append.
    #[test]
    fn the_footprint_is_idempotent() {
        assert_eq!(steps(), steps());
    }
}
