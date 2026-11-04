//! Versioned, bounded tab transport. Parsing never probes paths or edits a view.

use crate::detached_view::DetachedView;
use std::{collections::HashMap, path::PathBuf};
use toml::Value;

pub const MAX_BYTES: usize = 1024 * 1024;
pub const MAX_TABS: usize = 1024;
pub const TIMEOUT_MS: u64 = 8_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Moved,
    Bookmarked,
    Rejected,
}

#[derive(Debug)]
pub struct Offer {
    pub id: String,
    pub expires: u64,
    pub at: (i32, i32),
    pub data: DetachedView,
}

#[derive(Debug)]
pub enum Packet {
    Offer(Offer),
    Reply { id: String, outcome: Outcome },
}

fn identifier(id: &str) -> bool {
    !id.is_empty() && id.len() <= 80 && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

#[cfg(windows)]
fn native_units(path: &std::path::Path) -> Option<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    Some(path.as_os_str().encode_wide().collect())
}
#[cfg(not(windows))]
fn native_units(path: &std::path::Path) -> Option<Vec<u16>> {
    Some(path.to_str()?.encode_utf16().collect())
}
#[cfg(windows)]
fn native_path(units: &[u16]) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    Some(std::ffi::OsString::from_wide(units).into())
}
#[cfg(not(windows))]
fn native_path(units: &[u16]) -> Option<PathBuf> {
    Some(String::from_utf16(units).ok()?.into())
}

impl Packet {
    pub fn encode(&self) -> Option<String> {
        let mut table = toml::Table::new();
        table.insert("version".into(), 2.into());
        match self {
            Self::Offer(offer) => {
                if !identifier(&offer.id)
                    || offer.data.dirs.is_empty()
                    || offer.data.dirs.len() > MAX_TABS
                    || offer.data.dirs.len() != offer.data.view.tabs.len()
                {
                    return None;
                }
                table.insert("kind".into(), "offer".into());
                table.insert("id".into(), offer.id.clone().into());
                table.insert("expires".into(), i64::try_from(offer.expires).ok()?.into());
                table.insert("x".into(), offer.at.0.into());
                table.insert("y".into(), offer.at.1.into());
                let mut view = offer.data.view.clone();
                for tab in &mut view.tabs {
                    tab.path.clear();
                }
                table.insert("view".into(), Value::try_from(view).ok()?);
                let paths = offer
                    .data
                    .dirs
                    .iter()
                    .map(|p| {
                        let units = native_units(p)?;
                        if units.is_empty() || units.len() > 32767 || units.contains(&0) {
                            return None;
                        }
                        Some(Value::Array(
                            units
                                .into_iter()
                                .map(|n| Value::Integer(i64::from(n)))
                                .collect(),
                        ))
                    })
                    .collect::<Option<Vec<_>>>()?;
                table.insert("paths".into(), Value::Array(paths));
            }
            Self::Reply { id, outcome } => {
                if !identifier(id) {
                    return None;
                }
                table.insert("kind".into(), "reply".into());
                table.insert("id".into(), id.clone().into());
                table.insert(
                    "outcome".into(),
                    match outcome {
                        Outcome::Moved => "moved",
                        Outcome::Bookmarked => "bookmarked",
                        Outcome::Rejected => "rejected",
                    }
                    .into(),
                );
            }
        }
        let text = toml::to_string(&table).ok()?;
        (text.len() <= MAX_BYTES).then_some(text)
    }

    pub fn decode(text: &str) -> Option<Self> {
        if text.len() > MAX_BYTES {
            return None;
        }
        let mut t: toml::Table = toml::from_str(text).ok()?;
        if t.remove("version")?.as_integer()? != 2 {
            return None;
        }
        let id = t.remove("id")?.as_str()?.to_owned();
        if !identifier(&id) {
            return None;
        }
        match t.remove("kind")?.as_str()? {
            "reply" => {
                let outcome = match t.remove("outcome")?.as_str()? {
                    "moved" => Outcome::Moved,
                    "bookmarked" => Outcome::Bookmarked,
                    "rejected" => Outcome::Rejected,
                    _ => return None,
                };
                t.is_empty().then_some(Self::Reply { id, outcome })
            }
            "offer" => {
                let expires = u64::try_from(t.remove("expires")?.as_integer()?).ok()?;
                let x = i32::try_from(t.remove("x")?.as_integer()?).ok()?;
                let y = i32::try_from(t.remove("y")?.as_integer()?).ok()?;
                let raw_view = t.remove("view")?;
                let view: favnyr_core::workspace::ViewState = raw_view.clone().try_into().ok()?;
                if view.tabs.is_empty()
                    || view.tabs.len() > MAX_TABS
                    || view.active_tab >= view.tabs.len()
                    || Value::try_from(&view).ok()? != raw_view
                {
                    return None;
                }
                let mut normalized = view.clone();
                normalized.sanitize();
                if normalized != view
                    || view.tabs.iter().any(|t| !t.path.is_empty())
                    || view
                        .tab_groups
                        .entries()
                        .iter()
                        .any(|g| g.name.trim().is_empty())
                {
                    return None;
                }
                let raw_paths = t.remove("paths")?;
                let paths = raw_paths.as_array()?;
                if paths.len() != view.tabs.len() || !t.is_empty() {
                    return None;
                }
                let dirs = paths
                    .iter()
                    .map(|path| {
                        let array = path.as_array()?;
                        if array.is_empty() || array.len() > 32767 {
                            return None;
                        }
                        let units = array
                            .iter()
                            .map(|v| u16::try_from(v.as_integer()?).ok())
                            .collect::<Option<Vec<_>>>()?;
                        if units.contains(&0) {
                            return None;
                        }
                        native_path(&units)
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some(Self::Offer(Offer {
                    id,
                    expires,
                    at: (x, y),
                    data: DetachedView {
                        view,
                        dirs,
                        position: None,
                    },
                }))
            }
            _ => None,
        }
    }
}

/// Keep completed replies until their offers can no longer be accepted.
/// Capacity rejects new requests instead of evicting a live replay guard.
#[derive(Default)]
pub struct Receipts(HashMap<(isize, String), (u64, Outcome)>);
impl Receipts {
    pub fn lookup(&mut self, sender: isize, id: &str, now: u64) -> Option<Outcome> {
        self.0.retain(|_, (expires, _)| *expires >= now);
        self.0
            .get(&(sender, id.to_owned()))
            .map(|(_, outcome)| *outcome)
    }
    pub fn accepts(&self, expires: u64, now: u64) -> bool {
        self.0.len() < 256 && expires >= now && expires <= now.saturating_add(TIMEOUT_MS)
    }
    pub fn record(&mut self, sender: isize, offer: &Offer, outcome: Outcome) {
        self.0
            .insert((sender, offer.id.clone()), (offer.expires, outcome));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use favnyr_core::workspace::{TabState, ViewState};

    fn offer() -> Offer {
        let tab: TabState = toml::from_str("path = 'folder01'").unwrap();
        let mut view = ViewState {
            tab_groups: Default::default(),
            stretch: 1.0,
            active_tab: 1,
            tabs: vec![tab.clone(), tab],
            columns: favnyr_core::columns::default_columns(),
            tab_bar_mode: 2,
            tab_bar_restore_mode: 2,
            vbar_width: 137.5,
        };
        let group = view
            .tab_groups
            .create(&mut view.tabs, &mut view.active_tab, &[0, 1], "Group 01", 3)
            .unwrap();
        view.tab_groups.set_collapsed(&group, true).unwrap();
        Offer {
            id: "a1-b2".into(),
            expires: 8000,
            at: (-1200, 435),
            data: DetachedView {
                view,
                dirs: vec!["folder01".into(), "folder02/\u{6587}\u{4ef6}".into()],
                position: None,
            },
        }
    }

    fn encoded() -> toml::Table {
        toml::from_str(&Packet::Offer(offer()).encode().unwrap()).unwrap()
    }

    #[test]
    fn offer_round_trip_preserves_native_paths_groups_active_and_settings() {
        let original = offer();
        let text = Packet::Offer(offer()).encode().unwrap();
        let Packet::Offer(decoded) = Packet::decode(&text).unwrap() else {
            panic!("expected offer")
        };
        assert_eq!(decoded.id, original.id);
        assert_eq!(decoded.at, original.at);
        assert_eq!(decoded.expires, original.expires);
        assert_eq!(decoded.data.dirs, original.data.dirs);
        let mut expected = original.data.view;
        for tab in &mut expected.tabs {
            tab.path.clear();
        }
        assert_eq!(decoded.data.view, expected);
    }

    #[test]
    fn malformed_metadata_is_rejected_as_a_whole_without_workspace_repair() {
        let mutations: Vec<(&str, Value)> = vec![
            ("version", 3.into()),
            ("expires", (-1).into()),
            ("x", i64::MAX.into()),
            ("id", "invalid id".into()),
            ("extra", true.into()),
            ("paths", Value::Array(vec![])),
            ("paths", Value::Array(vec![Value::Array(vec![0.into()]); 2])),
            (
                "paths",
                Value::Array(vec![Value::Array(vec![65536.into()]); 2]),
            ),
        ];
        for (key, value) in mutations {
            let mut table = encoded();
            table.insert(key.into(), value);
            assert!(
                Packet::decode(&toml::to_string(&table).unwrap()).is_none(),
                "{key}"
            );
        }
        for (key, value) in [
            ("active_tab", 99.into()),
            ("tab_bar_mode", 9.into()),
            ("vbar_width", (-1.0).into()),
            ("stretch", 0.0.into()),
            ("unknown", true.into()),
        ] {
            let mut table = encoded();
            table
                .get_mut("view")
                .unwrap()
                .as_table_mut()
                .unwrap()
                .insert(key.into(), value);
            assert!(
                Packet::decode(&toml::to_string(&table).unwrap()).is_none(),
                "{key}"
            );
        }
        let mut table = encoded();
        table["view"]["tabs"][0]["tab_group_id"] = "missing".into();
        assert!(Packet::decode(&toml::to_string(&table).unwrap()).is_none());
    }

    #[test]
    fn transport_bounds_and_reply_kinds_are_explicit() {
        assert!(Packet::decode(&" ".repeat(MAX_BYTES + 1)).is_none());
        let mut o = offer();
        o.data.dirs[0] = "a".repeat(32768).into();
        assert!(Packet::Offer(o).encode().is_none());
        let mut o = offer();
        o.data.dirs.clear();
        assert!(Packet::Offer(o).encode().is_none());
        for outcome in [Outcome::Moved, Outcome::Bookmarked, Outcome::Rejected] {
            let text = Packet::Reply {
                id: "a1".into(),
                outcome,
            }
            .encode()
            .unwrap();
            let Packet::Reply {
                id,
                outcome: received,
            } = Packet::decode(&text).unwrap()
            else {
                panic!("expected reply")
            };
            assert_eq!(id, "a1");
            assert_eq!(received, outcome);
        }
    }

    #[test]
    fn replay_guard_is_scoped_to_sender_bounded_and_expires_after_acceptance() {
        let mut receipts = Receipts::default();
        let mut o = offer();
        receipts.record(1, &o, Outcome::Moved);
        assert_eq!(receipts.lookup(1, &o.id, 100), Some(Outcome::Moved));
        assert_eq!(receipts.lookup(2, &o.id, 100), None);
        assert!(!receipts.accepts(99, 100));
        assert!(!receipts.accepts(100 + TIMEOUT_MS + 1, 100));
        for n in 1..256 {
            o.id = format!("{n:x}");
            receipts.record(1, &o, Outcome::Rejected);
        }
        assert!(!receipts.accepts(8000, 100));
        assert_eq!(receipts.lookup(1, "a1-b2", 8000), Some(Outcome::Moved));
        assert_eq!(receipts.lookup(1, "a1-b2", 8001), None);
        assert!(receipts.accepts(9000, 8001));
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_preserve_unpaired_native_units() {
        use std::os::windows::ffi::OsStringExt;
        let mut o = offer();
        o.data.dirs[0] = std::ffi::OsString::from_wide(&[102, 0xd800, 48]).into();
        let path = o.data.dirs[0].clone();
        let text = Packet::Offer(o).encode().unwrap();
        let Packet::Offer(decoded) = Packet::decode(&text).unwrap() else {
            panic!("expected offer")
        };
        assert_eq!(decoded.data.dirs[0], path);
    }
}
