use gbn_bridge_protocol::{BootstrapProgress, BootstrapProgressStage};

use crate::publisher_client::PublisherClient;
use crate::RuntimeResult;

#[derive(Debug, Clone, Default)]
pub struct ProgressReporter {
    emitted: Vec<BootstrapProgress>,
}

impl ProgressReporter {
    pub fn report(
        &mut self,
        publisher_client: &mut PublisherClient,
        chain_id: &str,
        reporter_id: &str,
        bootstrap_session_id: &str,
        stage: BootstrapProgressStage,
        active_bridge_count: u16,
        reported_at_ms: u64,
    ) -> RuntimeResult<()> {
        let progress = BootstrapProgress {
            bootstrap_session_id: bootstrap_session_id.to_string(),
            reporter_id: reporter_id.to_string(),
            stage,
            active_bridge_count,
            reported_at_ms,
        };

        publisher_client.report_progress(chain_id, progress.clone())?;
        self.emitted.push(progress);
        Ok(())
    }

    pub fn emitted(&self) -> &[BootstrapProgress] {
        &self.emitted
    }
}
