//! Names for the addresses the screens show, looked up when a screen asks.
//!
//! Every screen that shows an IP address also shows the name it resolves to,
//! so screens ask often and for the same addresses. Answers are remembered
//! for a while (a name for 10 minutes, "no name" for 2), and an address that
//! is already being looked up is not asked about a second time: whoever asks
//! meanwhile waits for that answer.

use super::names;
use crate::error::{Error, Result};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::watch;
use tokio::time::Instant;

/// Most addresses one request may hold. A screen's store sends them in
/// batches of this size; a scan of a /22 finds at most 1022 devices.
pub const MAX_BATCH: usize = 256;
/// How long a found name is used before it is looked up again.
const FOUND_FOR: Duration = Duration::from_secs(10 * 60);
/// How long "no name" is believed: a device may announce a name a little later.
const NONE_FOR: Duration = Duration::from_secs(2 * 60);
/// Most addresses remembered at once. A home network has far fewer; this only
/// stops the memory growing without end if something asks for a great many.
const MAX_KEPT: usize = 4096;

/// The name for one address, as the screens get it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedName {
    /// Exactly as it was asked for.
    pub address: String,
    /// Already tidied for showing ("nas.lan" gives "nas"). None when no name
    /// was found, and for anything that isn't an IPv4 address.
    pub name: Option<String>,
}

/// Names for `addresses`, one answer per address in the same order. Only IPv4
/// addresses are looked up: a host that is already a name, or an IPv6
/// address, gets no name. At most `MAX_BATCH` addresses at a time. Takes
/// about 2.5 seconds at the most, and no time at all for addresses looked up
/// recently. Must run inside the tokio runtime.
pub async fn resolve_names(addresses: Vec<String>) -> Result<Vec<ResolvedName>> {
    static SHARED: OnceLock<Arc<NameCache>> = OnceLock::new();
    let cache = SHARED.get_or_init(|| Arc::new(NameCache::new(FOUND_FOR, NONE_FOR)));
    cache.resolve(&addresses, |ips| async move { names::discover(&ips).await }).await
}

enum Slot {
    /// Looked up; believed until `until`.
    Known { name: Option<String>, until: Instant },
    /// Being looked up. The value turns true once the answer is in `slots`.
    Asking(watch::Receiver<bool>),
}

/// What has been looked up, and what is being looked up right now.
pub(super) struct NameCache {
    slots: Mutex<HashMap<Ipv4Addr, Slot>>,
    found_for: Duration,
    none_for: Duration,
}

impl NameCache {
    pub(super) fn new(found_for: Duration, none_for: Duration) -> Self {
        Self { slots: Mutex::new(HashMap::new()), found_for, none_for }
    }

    fn slots(&self) -> std::sync::MutexGuard<'_, HashMap<Ipv4Addr, Slot>> {
        self.slots.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `resolve_names` with any way of finding names. `lookup` gets the
    /// addresses nobody is asking about yet (none twice), and is only called
    /// when there are some. It runs as a task of its own, so its answer is
    /// kept even if this call is given up on halfway.
    pub(super) async fn resolve<F, Fut>(self: &Arc<Self>, addresses: &[String], lookup: F) -> Result<Vec<ResolvedName>>
    where
        F: FnOnce(Vec<Ipv4Addr>) -> Fut,
        Fut: Future<Output = HashMap<Ipv4Addr, String>> + Send + 'static,
    {
        if addresses.len() > MAX_BATCH {
            return Err(Error::Invalid(format!(
                "Names can be looked up for at most {MAX_BATCH} addresses at a time, not {}.",
                addresses.len()
            )));
        }
        let wanted: Vec<Option<Ipv4Addr>> = addresses.iter().map(|a| a.trim().parse().ok()).collect();

        let mut waits = Vec::new();
        let mut to_ask = Vec::new();
        let (done, asking) = watch::channel(false);
        {
            let mut slots = self.slots();
            let now = Instant::now();
            slots.retain(|_, slot| !matches!(slot, Slot::Known { until, .. } if *until <= now));
            let mut seen = HashSet::new();
            for ip in wanted.iter().flatten().filter(|ip| seen.insert(**ip)) {
                match slots.get(ip) {
                    Some(Slot::Known { .. }) => {}
                    Some(Slot::Asking(other)) => waits.push(other.clone()),
                    None => to_ask.push(*ip),
                }
            }
            if !to_ask.is_empty() {
                if slots.len() + to_ask.len() > MAX_KEPT {
                    slots.retain(|_, slot| matches!(slot, Slot::Asking(_)));
                }
                for ip in &to_ask {
                    slots.insert(*ip, Slot::Asking(asking.clone()));
                }
                waits.push(asking);
            }
        }

        if !to_ask.is_empty() {
            // Made first, so that even a lookup that fails straight away lets its addresses go.
            let unfinished = Unfinished { cache: self.clone(), ips: to_ask.clone(), done: Some(done) };
            let answer = lookup(to_ask);
            tokio::spawn(async move {
                let found = answer.await;
                unfinished.finish(found);
            });
        }
        for mut wait in waits {
            // An error means the lookup ended without an answer: those addresses get no name this time.
            let _ = wait.wait_for(|done| *done).await;
        }

        let slots = self.slots();
        let name_of = |ip: &Option<Ipv4Addr>| match ip.and_then(|ip| slots.get(&ip)) {
            Some(Slot::Known { name, .. }) => name.clone(),
            _ => None,
        };
        Ok(addresses
            .iter()
            .zip(&wanted)
            .map(|(address, ip)| ResolvedName { address: address.clone(), name: name_of(ip) })
            .collect())
    }
}

/// One lookup that is under way. If it ends without an answer (it panicked,
/// or the runtime shut down), its addresses are let go, so the next request
/// asks again instead of waiting for ever.
struct Unfinished {
    cache: Arc<NameCache>,
    ips: Vec<Ipv4Addr>,
    done: Option<watch::Sender<bool>>,
}

impl Unfinished {
    fn finish(mut self, found: HashMap<Ipv4Addr, String>) {
        let now = Instant::now();
        {
            let mut slots = self.cache.slots();
            for ip in &self.ips {
                let name = found.get(ip).cloned();
                let until = now + if name.is_some() { self.cache.found_for } else { self.cache.none_for };
                slots.insert(*ip, Slot::Known { name, until });
            }
        }
        if let Some(done) = self.done.take() {
            done.send_replace(true);
        }
    }
}

impl Drop for Unfinished {
    fn drop(&mut self) {
        if self.done.is_none() {
            return;
        }
        let mut slots = self.cache.slots();
        for ip in &self.ips {
            if matches!(slots.get(ip), Some(Slot::Asking(_))) {
                slots.remove(ip);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn ip(text: &str) -> Ipv4Addr {
        text.parse().unwrap()
    }

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn answers(got: &[ResolvedName]) -> Vec<(&str, Option<&str>)> {
        got.iter().map(|r| (r.address.as_str(), r.name.as_deref())).collect()
    }

    /// A name service that knows two devices, counts every address it is
    /// asked about, and takes `delay` to answer.
    #[derive(Clone)]
    struct Service {
        asked: Arc<Mutex<Vec<Ipv4Addr>>>,
        calls: Arc<AtomicUsize>,
        delay: Duration,
    }

    impl Service {
        fn new(delay: Duration) -> Self {
            Self { asked: Arc::default(), calls: Arc::default(), delay }
        }

        fn lookup(&self) -> impl FnOnce(Vec<Ipv4Addr>) -> std::pin::Pin<Box<dyn Future<Output = HashMap<Ipv4Addr, String>> + Send>> {
            let me = self.clone();
            move |ips| {
                me.calls.fetch_add(1, Ordering::SeqCst);
                me.asked.lock().unwrap().extend(&ips);
                Box::pin(async move {
                    tokio::time::sleep(me.delay).await;
                    let known = [("192.168.68.10", "nas"), ("192.168.68.21", "homelab")];
                    known.into_iter().map(|(a, n)| (ip(a), n.to_string())).filter(|(a, _)| ips.contains(a)).collect()
                })
            }
        }

        fn asked(&self) -> Vec<Ipv4Addr> {
            let mut asked = self.asked.lock().unwrap().clone();
            asked.sort();
            asked
        }
    }

    fn cache(found_for: Duration, none_for: Duration) -> Arc<NameCache> {
        Arc::new(NameCache::new(found_for, none_for))
    }

    const LONG: Duration = Duration::from_secs(600);

    #[tokio::test]
    async fn answers_each_address_in_order_and_remembers() {
        let (cache, service) = (cache(LONG, LONG), Service::new(Duration::ZERO));
        let asked = strings(&["192.168.68.10", "192.168.68.99", " 192.168.68.21 ", "192.168.68.10"]);
        let got = cache.resolve(&asked, service.lookup()).await.unwrap();
        assert_eq!(
            answers(&got),
            [("192.168.68.10", Some("nas")), ("192.168.68.99", None), (" 192.168.68.21 ", Some("homelab")), ("192.168.68.10", Some("nas"))]
        );
        // The address given twice was asked about once.
        assert_eq!(service.asked(), [ip("192.168.68.10"), ip("192.168.68.21"), ip("192.168.68.99")]);

        // Asked again: answered from memory, "no name" included, without a lookup.
        let again = cache.resolve(&asked, service.lookup()).await.unwrap();
        assert_eq!(again, got);
        assert_eq!(service.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn names_and_odd_text_are_not_looked_up() {
        let (cache, service) = (cache(LONG, LONG), Service::new(Duration::ZERO));
        let asked = strings(&["nas.lan", "fe80::1", "", "192.168.68", "192.168.68.300"]);
        let got = cache.resolve(&asked, service.lookup()).await.unwrap();
        assert!(got.iter().all(|r| r.name.is_none()));
        assert_eq!(got.iter().map(|r| r.address.as_str()).collect::<Vec<_>>(), asked);
        assert_eq!(service.calls.load(Ordering::SeqCst), 0, "nothing to look up");
        assert!(cache.resolve(&[], service.lookup()).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn no_name_is_asked_again_sooner_than_a_name() {
        let (cache, service) = (cache(LONG, Duration::from_millis(30)), Service::new(Duration::ZERO));
        let asked = strings(&["192.168.68.10", "192.168.68.99"]);
        cache.resolve(&asked, service.lookup()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;
        let got = cache.resolve(&asked, service.lookup()).await.unwrap();
        assert_eq!(answers(&got), [("192.168.68.10", Some("nas")), ("192.168.68.99", None)]);
        // Only the address without a name was looked up a second time.
        assert_eq!(service.asked(), [ip("192.168.68.10"), ip("192.168.68.99"), ip("192.168.68.99")]);
    }

    #[tokio::test]
    async fn a_found_name_is_looked_up_again_once_it_is_old() {
        let (cache, service) = (cache(Duration::from_millis(30), LONG), Service::new(Duration::ZERO));
        let asked = strings(&["192.168.68.10"]);
        cache.resolve(&asked, service.lookup()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;
        cache.resolve(&asked, service.lookup()).await.unwrap();
        assert_eq!(service.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn screens_asking_at_the_same_time_share_one_lookup_per_address() {
        let (cache, service) = (cache(LONG, LONG), Service::new(Duration::from_millis(80)));
        let first = strings(&["192.168.68.10", "192.168.68.99"]);
        let second = strings(&["192.168.68.99", "192.168.68.21", "192.168.68.10"]);
        let started = std::time::Instant::now();
        let (a, b) = tokio::join!(cache.resolve(&first, service.lookup()), cache.resolve(&second, service.lookup()));
        assert_eq!(answers(&a.unwrap()), [("192.168.68.10", Some("nas")), ("192.168.68.99", None)]);
        assert_eq!(
            answers(&b.unwrap()),
            [("192.168.68.99", None), ("192.168.68.21", Some("homelab")), ("192.168.68.10", Some("nas"))]
        );
        // Each address once: the second request only asked about the one the first didn't have.
        assert_eq!(service.asked(), [ip("192.168.68.10"), ip("192.168.68.21"), ip("192.168.68.99")]);
        assert_eq!(service.calls.load(Ordering::SeqCst), 2);
        // The two lookups ran side by side, not one after the other.
        assert!(started.elapsed() < Duration::from_millis(155), "{:?}", started.elapsed());
    }

    #[tokio::test]
    async fn a_request_given_up_on_still_leaves_its_answer() {
        let (cache, service) = (cache(LONG, LONG), Service::new(Duration::from_millis(40)));
        let asked = strings(&["192.168.68.21"]);
        let cut_short = tokio::time::timeout(Duration::from_millis(5), cache.resolve(&asked, service.lookup())).await;
        assert!(cut_short.is_err());
        // The next request waits for the lookup already under way instead of starting another.
        let got = cache.resolve(&asked, service.lookup()).await.unwrap();
        assert_eq!(answers(&got), [("192.168.68.21", Some("homelab"))]);
        assert_eq!(service.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_lookup_that_fails_is_not_waited_for_or_remembered() {
        let (cache, service) = (cache(LONG, LONG), Service::new(Duration::ZERO));
        let asked = strings(&["192.168.68.10"]);
        let broken = |ips: Vec<Ipv4Addr>| async move {
            assert!(ips.is_empty(), "the name service fell over (this is part of a test)");
            HashMap::new()
        };
        let got = cache.resolve(&asked, broken).await.unwrap();
        assert_eq!(answers(&got), [("192.168.68.10", None)]);
        // Not stuck as "being looked up", and the failure isn't taken as "no name".
        let got = cache.resolve(&asked, service.lookup()).await.unwrap();
        assert_eq!(answers(&got), [("192.168.68.10", Some("nas"))]);
        assert_eq!(service.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn too_many_addresses_at_once_are_refused_in_plain_words() {
        let (cache, service) = (cache(LONG, LONG), Service::new(Duration::ZERO));
        let many: Vec<String> = (0..=MAX_BATCH).map(|i| format!("10.0.{}.{}", i / 250, i % 250 + 1)).collect();
        match cache.resolve(&many, service.lookup()).await {
            Err(Error::Invalid(message)) => assert!(message.contains("at most 256"), "{message}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(service.calls.load(Ordering::SeqCst), 0);
        assert_eq!(cache.resolve(&many[..MAX_BATCH], service.lookup()).await.unwrap().len(), MAX_BATCH);
    }

    #[test]
    fn the_command_can_move_between_threads() {
        fn is_send<T: Send>(_: &T) {}
        is_send(&resolve_names(Vec::new()));
    }
}
