//! Version semantics for the update check.
//!
//! The rules follow Magpie's: a release tag may carry a leading `v`, build
//! metadata after `+` plays no part in the order, and a pre-release sorts below
//! the release it leads up to.

use std::cmp::Ordering;

/// A `MAJOR.MINOR.PATCH` version with an optional pre-release suffix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    numbers: [u64; 3],
    pre: String,
}

impl Version {
    /// Parse `v?MAJOR.MINOR.PATCH(-pre)?(+build)?`.
    ///
    /// Build metadata is dropped and the pre-release suffix is kept verbatim
    /// for [`Version::cmp`]. Anything that is not three numeric components is
    /// rejected, which is what keeps `dev` and `git describe` strings out.
    pub fn parse(text: &str) -> Option<Version> {
        let text = text.trim();
        let text = text.strip_prefix('v').unwrap_or(text);
        let text = text.split_once('+').map_or(text, |(head, _)| head);
        let (core, pre) = text.split_once('-').unwrap_or((text, ""));

        let mut numbers = [0; 3];
        let mut parts = core.split('.');
        for slot in numbers.iter_mut() {
            *slot = parts.next()?.parse().ok()?;
        }
        if parts.next().is_some() {
            return None;
        }

        Some(Version {
            numbers,
            pre: pre.to_string(),
        })
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.numbers
            .cmp(&other.numbers)
            .then_with(|| compare_pre(&self.pre, &other.pre))
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Order two pre-release suffixes the way semver §11 does: no pre-release comes
/// last, otherwise identifier by identifier, numbers as numbers, a number
/// before a word, and a shorter list before a longer one it starts.
fn compare_pre(a: &str, b: &str) -> Ordering {
    match (a.is_empty(), b.is_empty()) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        (false, false) => {}
    }

    let (mut left, mut right) = (a.split('.'), b.split('.'));
    loop {
        match (left.next(), right.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                let order = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(m), Ok(n)) => m.cmp(&n),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => x.cmp(y),
                };
                if order != Ordering::Equal {
                    return order;
                }
            }
        }
    }
}

/// Whether `candidate` is a later release than `current`.
///
/// Unparseable input on either side means no update, so a development build can
/// never be talked into replacing itself.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (Version::parse(candidate), Version::parse(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

/// Whether this build came from a release tag rather than a working tree.
///
/// `dev` and `git describe` strings (`0.1.0-3-gabc1234`) describe a build from
/// source; only releases update themselves.
pub fn is_released(version: &str) -> bool {
    let Some(parsed) = Version::parse(version) else {
        return false;
    };
    let pre = &parsed.pre;
    !pre.contains("dirty") && !looks_like_describe(pre)
}

/// Whether a pre-release suffix is `git describe`'s `<commits>-g<hash>`.
fn looks_like_describe(pre: &str) -> bool {
    let Some((commits, rest)) = pre.split_once('-') else {
        return false;
    };
    let Some(hash) = rest.strip_prefix('g') else {
        return false;
    };
    !commits.is_empty()
        && commits.bytes().all(|byte| byte.is_ascii_digit())
        && !hash.is_empty()
        && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The version this build reports everywhere else.
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// What this build actually is: [`current_version`] for a release, `git describe`
/// for one built from a working tree.
pub fn build_stamp() -> &'static str {
    env!("AIDGET_BUILD_STAMP")
}
