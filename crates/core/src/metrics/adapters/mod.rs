//! Where a source's numbers come from: one adapter per service, one file each
//! (`adapters/<kind>.rs`, docs/METRICS.md). An adapter says what it is set up
//! with and what it reports ([`SourceSpec`]), and fetches days of points; the
//! rest — keys, snapshots, periods, comparisons — is `metrics`' own.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use crate::protocol::metrics::{FetchError, Point, SourceKind, SourceSpec};

pub mod fake;

/// What a fetch is given.
pub struct Fetch<'a> {
    /// The connection's fields that aren't secret.
    pub settings: &'a BTreeMap<String, String>,
    /// Its secret fields, from the Keychain. An adapter sends them to its
    /// service and nowhere else: not in a point, an error or a log.
    pub keys: &'a BTreeMap<String, String>,
    /// Today, the daemon machine's local day, `YYYY-MM-DD`.
    pub today: &'a str,
    /// How many days back to ask for (from today): more on the first fetch.
    pub days: u32,
    pub client: &'a reqwest::Client,
}

pub type Fetching<'a> = Pin<Box<dyn Future<Output = Result<Vec<Point>, FetchError>> + Send + 'a>>;

/// A service numbers come from.
pub trait Adapter: Send + Sync {
    fn spec(&self) -> SourceSpec;
    /// The points of the days asked for — a day the service doesn't know yet
    /// (App Store's today) left out, not zero.
    fn fetch<'a>(&'a self, fetch: Fetch<'a>) -> Fetching<'a>;
}

/// The adapter for `kind`, when there is one yet.
pub fn adapter(kind: SourceKind) -> Option<Box<dyn Adapter>> {
    match kind {
        SourceKind::Fake => Some(Box::new(fake::Fake)),
        // Their own tickets: `adapters/posthog.rs`, `github.rs`, `app_store.rs`.
        SourceKind::Posthog | SourceKind::Github | SourceKind::AppStore => None,
    }
}

/// The kinds a screen may offer: every one with an adapter. The fake one
/// only when `EQUIPE_METRICS_FAKE` is set, or in tests.
pub fn offered(with_fake: bool) -> Vec<SourceSpec> {
    [SourceKind::Posthog, SourceKind::Github, SourceKind::AppStore, SourceKind::Fake]
        .into_iter()
        .filter(|k| with_fake || *k != SourceKind::Fake)
        .filter_map(adapter)
        .map(|a| a.spec())
        .collect()
}
