use gbn_bridge_protocol::BridgeData;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardedFrame {
    pub frame: BridgeData,
}

#[derive(Debug, Clone, Default)]
pub struct PayloadForwarder {
    forwarded: Vec<ForwardedFrame>,
}

impl PayloadForwarder {
    pub fn forward(&mut self, frame: BridgeData) {
        self.forwarded.push(ForwardedFrame { frame });
    }

    pub fn forwarded(&self) -> &[ForwardedFrame] {
        &self.forwarded
    }
}
