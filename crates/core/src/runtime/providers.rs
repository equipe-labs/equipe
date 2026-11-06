//! The model providers the person connects (`crate::model_providers`), for
//! every screen: each one, where its key comes from — never the key — and
//! the models it lists, looked at off the loop; and the provider a member's
//! model runs on, for its start and its turns.

use equipe_protocol::{ProviderStatus, ProviderTest};

use super::*;
use crate::model_providers as mp;

/// What a provider's listing found, as the screens are told it.
#[derive(Debug, Clone, Default)]
pub(super) struct Listed {
    pub test: ProviderTest,
    /// Its models' own ids, best first.
    pub models: Vec<String>,
}

/// The daemon's environment, where a provider's key may come from.
fn env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

impl Runtime {
    /// The providers connected here, with what each lists.
    fn provider_statuses(&self) -> Vec<ProviderStatus> {
        mp::entries(&self.config.home, env)
            .into_iter()
            .map(|e| {
                let listed = self.providers.get(&e.id).cloned().unwrap_or_default();
                let models = listed
                    .models
                    .iter()
                    .map(|m| {
                        let shown = mp::display(m);
                        ModelChoice { id: mp::model_ref(&e.id, m), name: (shown != m).then(|| shown.to_owned()), ..ModelChoice::default() }
                    })
                    .collect();
                ProviderStatus { id: e.id, preset: e.preset, name: e.name, base_url: e.base_url, key: e.key, removable: e.removable, models, test: listed.test }
            })
            .collect()
    }

    pub(super) fn publish_providers(&mut self) {
        let providers = self.provider_statuses();
        let codex = codex_app::find(&self.config.codex_binary).is_some();
        self.publish(Event::Providers { providers, codex });
    }

    /// A screen asks: the providers as they stand, and each one with a key
    /// that hasn't been looked at, listed now.
    pub(super) fn providers_asked(&mut self) {
        self.publish_providers();
        let untested: Vec<String> = mp::entries(&self.config.home, env)
            .into_iter()
            .filter(|e| e.key != equipe_protocol::ProviderKey::None)
            .filter(|e| self.providers.get(&e.id).is_none_or(|l| l.test == ProviderTest::Untested))
            .map(|e| e.id)
            .collect();
        for id in untested {
            self.test_provider(id);
        }
    }

    /// List provider `id`'s models off the loop (`GET /models`).
    pub(super) fn test_provider(&mut self, id: String) {
        let Some(entry) = mp::entries(&self.config.home, env).into_iter().find(|e| e.id == id) else {
            return;
        };
        let cfg = match mp::config(&self.config.home, &id, env) {
            Ok(cfg) => cfg,
            Err(err) => return self.provider_listed(id, Err(err)),
        };
        let listed = self.providers.entry(id.clone()).or_default();
        if listed.test == ProviderTest::Testing {
            return;
        }
        listed.test = ProviderTest::Testing;
        self.publish(Event::ProviderTested { id: id.clone(), test: ProviderTest::Testing });
        let internal = self.internal.clone();
        tokio::spawn(async move {
            let result = match tokio::time::timeout(SERVER_WAIT, mp::list(&cfg, &entry.id)).await {
                Ok(result) => result,
                Err(_) => Err(crate::nemotron::ReviewError::Timeout),
            };
            let _ = internal.send(Internal::ProviderListed { id, result });
        });
    }

    /// Provider `id`'s models as just listed — or why not, in plain words.
    pub(super) fn provider_listed(&mut self, id: String, result: Result<Vec<String>, crate::nemotron::ReviewError>) {
        let entry = mp::entries(&self.config.home, env).into_iter().find(|e| e.id == id);
        let (name, base_url) = entry.map_or_else(|| (id.clone(), String::new()), |e| (e.name, e.base_url));
        let listed = match result {
            Ok(models) => Listed { test: ProviderTest::Ok { count: models.len() }, models },
            Err(err) => {
                crate::daemon_log!("equipe: provider {id}: models not listed ({})", err.kind());
                Listed { test: ProviderTest::Failed { why: mp::why(&err, &name, &base_url, self.config.lang), problem: Some(mp::problem(&err)) }, models: Vec::new() }
            }
        };
        let test = listed.test.clone();
        self.providers.insert(id.clone(), listed);
        self.publish(Event::ProviderTested { id, test });
        self.publish_providers();
    }

    /// The person connects a provider from a screen: kept, then listed.
    pub(super) fn add_provider(&mut self, preset: String, base_url: Option<String>, key: Option<String>) {
        let name = mp::preset(&preset).map_or_else(|| preset.clone(), |p| p.name.to_owned());
        match mp::add(&self.config.home, &preset, base_url.as_deref(), key.as_deref()) {
            Ok(id) => {
                crate::daemon_log!("equipe: provider {id} connected");
                self.providers.remove(&id);
                if id == mp::NEBIUS {
                    self.config.nemotron = crate::nemotron::Config::from_env_or_saved(&self.config.home).ok();
                    self.publish_nebius();
                }
                self.publish_providers();
                self.test_provider(id);
            }
            Err(err) => {
                let why = err.why(self.config.lang, &name);
                self.publish(Event::ProviderTested { id: preset, test: ProviderTest::Failed { why: why.clone(), problem: Some(err.problem()) } });
                self.error(None, why, None);
            }
        }
    }

    /// The person removes a provider: it and its saved key forgotten.
    pub(super) fn remove_provider(&mut self, id: String) {
        if let Err(err) = mp::remove(&self.config.home, &id) {
            let name = self.provider_name(&id);
            self.error(None, say!(self, "provider.add.io", provider = name, why = err), None);
        }
        self.providers.remove(&id);
        crate::daemon_log!("equipe: provider {id} removed");
        if id == mp::NEBIUS {
            self.config.nemotron = crate::nemotron::Config::from_env_or_saved(&self.config.home).ok();
            self.publish_nebius();
        }
        self.publish_providers();
    }

    /// A provider's name as the person knows it: its entry's, its preset's, else its id.
    pub(super) fn provider_name(&self, id: &str) -> String {
        mp::entries(&self.config.home, env)
            .into_iter()
            .find(|e| e.id == id)
            .map(|e| e.name)
            .or_else(|| mp::preset(id).map(|p| p.name.to_owned()))
            .unwrap_or_else(|| id.to_owned())
    }

    /// The provider a member's `model` runs on: the one it names
    /// (`@openrouter/…`), else — an open model as before — the
    /// environment's server or Nebius (`None` when there is neither). An
    /// error, in words, when the one it names isn't connected.
    pub(super) fn open_config(&self, model: Option<&str>) -> Result<Option<crate::nemotron::Config>, String> {
        match model.and_then(mp::split_ref) {
            Some((id, _)) => mp::config(&self.config.home, id, env).map(Some).map_err(|_| say!(self, "error.provider_not_connected", provider = self.provider_name(id))),
            None => Ok(self.config.nemotron.clone()),
        }
    }

    /// A model named for a provider, as a member's notice says it: the
    /// model's own id, and the provider's name.
    pub(super) fn on_provider(&self, model: &str) -> Option<(String, String)> {
        let (id, _) = mp::split_ref(model)?;
        Some((mp::display(model).to_owned(), self.provider_name(id)))
    }
}
