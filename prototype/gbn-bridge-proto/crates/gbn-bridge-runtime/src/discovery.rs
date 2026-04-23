use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiscoveryHintSource {
    SeedCatalog,
    WeakDiscovery,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryHint {
    pub bridge_id: String,
    pub host: String,
    pub port: u16,
    pub observed_at_ms: u64,
    pub source: DiscoveryHintSource,
}

impl DiscoveryHint {
    pub fn is_fresh(&self, max_hint_age_ms: u64, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.observed_at_ms) <= max_hint_age_ms
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeakDiscoveryConfig {
    pub enabled: bool,
    pub max_hint_age_ms: u64,
}

impl Default for WeakDiscoveryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_hint_age_ms: 300_000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WeakDiscoveryState {
    config: WeakDiscoveryConfig,
    hints: BTreeMap<String, DiscoveryHint>,
}

impl Default for WeakDiscoveryState {
    fn default() -> Self {
        Self {
            config: WeakDiscoveryConfig::default(),
            hints: BTreeMap::new(),
        }
    }
}

impl WeakDiscoveryState {
    pub fn config(&self) -> WeakDiscoveryConfig {
        self.config
    }

    pub fn enabled(&self) -> bool {
        self.config.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.config.enabled = enabled;
    }

    pub fn len(&self) -> usize {
        self.hints.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hints.is_empty()
    }

    pub fn hint(&self, bridge_id: &str) -> Option<&DiscoveryHint> {
        self.hints.get(bridge_id)
    }

    pub fn ingest<I>(&mut self, hints: I) -> usize
    where
        I: IntoIterator<Item = DiscoveryHint>,
    {
        let mut inserted = 0;

        for hint in hints {
            if hint.port == 0 {
                continue;
            }

            let should_replace = match self.hints.get(&hint.bridge_id) {
                Some(existing) => hint.observed_at_ms > existing.observed_at_ms,
                None => true,
            };

            if should_replace {
                self.hints.insert(hint.bridge_id.clone(), hint);
                inserted += 1;
            }
        }

        inserted
    }

    pub fn snapshot_fresh(&self, now_ms: u64) -> Vec<DiscoveryHint> {
        if !self.config.enabled {
            return Vec::new();
        }

        let mut hints: Vec<_> = self
            .hints
            .values()
            .filter(|hint| hint.is_fresh(self.config.max_hint_age_ms, now_ms))
            .cloned()
            .collect();

        hints.sort_by(|left, right| {
            right
                .observed_at_ms
                .cmp(&left.observed_at_ms)
                .then_with(|| left.bridge_id.cmp(&right.bridge_id))
        });

        hints
    }
}
