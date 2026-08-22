//! Who something belongs to (#7744).
//!
//! An agent acts on behalf of someone, and whatever it creates -- a workflow,
//! a cron job, a skill, another agent -- belongs to that someone, not to the
//! agent. The agent is the executor; recording it as the owner is like
//! recording the printer as the author.
//!
//! Ownership is not always one human. A workflow built during a support shift
//! belongs to the support team; a compliance cron belongs to whoever holds
//! that duty this quarter; an ops runbook belongs to a role, and the people
//! filling it change. So the owner is a *principal*, not a user id -- the
//! shape every IAM system converges on, for the same reason: identity
//! outlives the individual.

use serde::{Deserialize, Serialize};

/// Whoever an agent was acting for when it created something.
///
/// Serialised as a tagged enum so the wire form says which kind it is rather
/// than leaving readers to guess from the shape of an opaque string:
///
/// ```toml
/// [owner]
/// kind = "group"
/// id = "support"
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Principal {
    /// A named user, matching `UserConfig::name`.
    ///
    /// The name rather than a UUID, because that is how users are addressed
    /// everywhere else in the configuration, and two identifier schemes for
    /// one entity is how references rot.
    User(String),
    /// A group, by its stable `GroupConfig::id`.
    ///
    /// The id and not the display name: a group is renamed far more often
    /// than it is dissolved, and a rename must not orphan what it owns.
    Group(String),
    /// A duty rather than a person or a list of them — "on-call",
    /// "compliance". Who fills it is answered elsewhere and changes; what the
    /// role owns does not.
    Role(String),
}

impl Principal {
    /// The identifier inside, whichever kind this is.
    ///
    /// Useful for logging and display. Deliberately *not* an equality
    /// shortcut: a user called `support` and a group called `support` are
    /// different owners, and comparing the bare strings would merge them.
    pub fn id(&self) -> &str {
        match self {
            Principal::User(id) | Principal::Group(id) | Principal::Role(id) => id,
        }
    }

    /// A short human-facing label, e.g. `group:support`.
    pub fn label(&self) -> String {
        let kind = match self {
            Principal::User(_) => "user",
            Principal::Group(_) => "group",
            Principal::Role(_) => "role",
        };
        format!("{kind}:{}", self.id())
    }
}

impl std::fmt::Display for Principal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wire_form_names_the_kind() {
        let json = serde_json::to_string(&Principal::Group("support".into())).unwrap();
        assert_eq!(json, r#"{"kind":"group","id":"support"}"#);
    }

    #[test]
    fn it_round_trips() {
        for p in [
            Principal::User("paco".into()),
            Principal::Group("support".into()),
            Principal::Role("on-call".into()),
        ] {
            let back: Principal =
                serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
            assert_eq!(back, p);
        }
    }

    /// The whole point of the enum: a user and a group can share a name and
    /// still be different owners. Collapsing them to their id would silently
    /// hand one's property to the other.
    #[test]
    fn a_user_and_a_group_with_the_same_name_are_different_owners() {
        let user = Principal::User("support".into());
        let group = Principal::Group("support".into());

        assert_ne!(user, group);
        assert_eq!(
            user.id(),
            group.id(),
            "the bare ids do collide — that is why kind is carried"
        );
        assert_ne!(user.label(), group.label());
    }

    #[test]
    fn it_reads_from_toml_as_an_operator_would_write_it() {
        #[derive(Deserialize)]
        struct Owned {
            owner: Principal,
        }
        let parsed: Owned = toml::from_str(
            r#"
            [owner]
            kind = "role"
            id = "compliance"
            "#,
        )
        .expect("a hand-written owner block must parse");

        assert_eq!(parsed.owner, Principal::Role("compliance".into()));
    }
}

#[cfg(test)]
mod ownership_survives_storage_tests {
    use super::*;

    /// An owner that does not survive being written and read back is not
    /// ownership, it is a label on a live object. The workflow this was built
    /// for lives in a file on disk and outlives the process that made it.
    #[test]
    fn an_owner_survives_a_toml_round_trip() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct Doc {
            name: String,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            owner: Option<Principal>,
        }

        let original = Doc {
            name: "nightly-report".to_string(),
            owner: Some(Principal::Group("support".into())),
        };
        let text = toml::to_string(&original).expect("serialises");
        let back: Doc = toml::from_str(&text).expect("parses back");

        assert_eq!(back, original);
    }

    /// Everything created before ownership existed has no owner, and must keep
    /// loading. A required field here would make every stored workflow
    /// unreadable on upgrade.
    #[test]
    fn a_document_without_an_owner_still_loads() {
        #[derive(Deserialize)]
        struct Doc {
            #[serde(default)]
            owner: Option<Principal>,
        }
        let back: Doc = toml::from_str(r#"name = "legacy""#).expect("must load without an owner");
        assert!(
            back.owner.is_none(),
            "absent means unowned, not a default owner"
        );
    }

    /// Absent must stay absent on write: a stranded `owner = ...` in an
    /// operator's file claims something the system does not know.
    #[test]
    fn an_absent_owner_writes_nothing() {
        #[derive(Serialize)]
        struct Doc {
            name: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            owner: Option<Principal>,
        }
        let text = toml::to_string(&Doc {
            name: "legacy".to_string(),
            owner: None,
        })
        .unwrap();

        assert!(!text.contains("owner"), "got: {text}");
    }
}
