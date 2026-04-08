#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibilitySurface {
    pub package_name: String,
    pub module_name: String,
}

impl Default for CompatibilitySurface {
    fn default() -> Self {
        Self {
            package_name: "metrust".to_string(),
            module_name: "metrust._metrust".to_string(),
        }
    }
}

