//! Screenshot runs only: `LODESTAR_METADATA_FIXTURES=<file.json>` answers the metadata worker's
//! requests from local files, so a headless screenshot can show Download metadata… without the
//! network. The JSON maps a request prefix to a file: `"https://…"` for a GET, `"POST https://…\n
//! <start of the body>"` for a POST; the longest matching prefix wins, anything else is a 404.
//! Ignored outside screenshot runs (`art::net::local_only`).

use super::{Http, HttpResponse};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

static ACTIVE: AtomicBool = AtomicBool::new(false);

/// The metadata worker answers from fixtures (lookups the user asks for may run offline).
pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

pub(crate) struct FixtureHttp {
    routes: Vec<(String, PathBuf)>,
}

/// The fixture answers named by `LODESTAR_METADATA_FIXTURES`, in a screenshot run.
pub(crate) fn from_env() -> Option<FixtureHttp> {
    if !crate::art::net::local_only() {
        return None;
    }
    let path = std::env::var_os("LODESTAR_METADATA_FIXTURES")?;
    let text = std::fs::read_to_string(&path)
        .inspect_err(|e| log::warn!("metadata fixtures {}: {e}", PathBuf::from(&path).display()))
        .ok()?;
    let map: std::collections::BTreeMap<String, String> =
        serde_json::from_str(&text).inspect_err(|e| log::warn!("metadata fixtures: {e}")).ok()?;
    ACTIVE.store(true, Ordering::Relaxed);
    Some(FixtureHttp { routes: map.into_iter().map(|(k, v)| (k, PathBuf::from(v))).collect() })
}

impl FixtureHttp {
    fn answer(&self, key: &str) -> HttpResponse {
        let best = self.routes.iter().filter(|(k, _)| key.starts_with(k.as_str())).max_by_key(|(k, _)| k.len());
        match best.and_then(|(_, p)| std::fs::read(p).ok()) {
            Some(body) => HttpResponse { status: 200, body },
            None => HttpResponse { status: 404, body: vec![] },
        }
    }
}

impl Http for FixtureHttp {
    fn get(&mut self, url: &str, _bearer: Option<&str>, _limit: u64) -> Result<HttpResponse, String> {
        Ok(self.answer(url))
    }

    fn post(
        &mut self,
        url: &str,
        _headers: &[(&str, String)],
        body: &str,
        _limit: u64,
    ) -> Result<HttpResponse, String> {
        Ok(self.answer(&format!("POST {url}\n{body}")))
    }
}
