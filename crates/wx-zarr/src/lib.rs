#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZarrDescriptor {
    pub group_name: String,
    pub chunk_shape: Vec<usize>,
}
