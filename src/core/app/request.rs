#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartupCheckCorrelation {
    pub generation: u64,
}

impl StartupCheckCorrelation {
    pub fn new(generation: u64) -> Self {
        Self { generation }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationLoadCorrelation {
    pub generation: u64,
    pub requested_thread_id: String,
}

impl ConversationLoadCorrelation {
    pub fn new(generation: u64, requested_thread_id: impl Into<String>) -> Self {
        Self {
            generation,
            requested_thread_id: requested_thread_id.into(),
        }
    }
}
