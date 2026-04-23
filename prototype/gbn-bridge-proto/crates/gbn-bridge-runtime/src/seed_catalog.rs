use crate::discovery::DiscoveryHint;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SeedCatalog {
    hints: Vec<DiscoveryHint>,
}

impl SeedCatalog {
    pub fn new(hints: Vec<DiscoveryHint>) -> Self {
        Self { hints }
    }

    pub fn len(&self) -> usize {
        self.hints.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hints.is_empty()
    }

    pub fn hints(&self) -> &[DiscoveryHint] {
        &self.hints
    }
}
