impl Lock {
    /// Choose the serialized representation for a standalone comparison of the same graph.
    #[must_use]
    pub fn with_build_encoding(mut self, encoding: BuildEncoding) -> Self {
        if let Some(builds) = &mut self.builds {
            builds.encoding = encoding;
        }
        self
    }
}
