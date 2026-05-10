#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppSnapshot {
    pub revision: u64,
}

impl AppSnapshot {
    pub fn initial() -> Self {
        Self { revision: 0 }
    }
}
