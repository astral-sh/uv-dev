/// A simple splitter that uses `memchr` to find the next delimiter.
pub(crate) struct MemchrSplitter<'a> {
    memchr: memchr::Memchr<'a>,
    haystack: &'a str,
    offset: usize,
}

impl<'a> MemchrSplitter<'a> {
    #[inline]
    pub(crate) fn split(haystack: &'a str, delimiter: u8) -> Self {
        Self {
            memchr: memchr::Memchr::new(delimiter, haystack.as_bytes()),
            haystack,
            offset: 0,
        }
    }
}

impl<'a> Iterator for MemchrSplitter<'a> {
    type Item = &'a str;

    #[inline(always)]
    #[expect(clippy::inline_always)]
    fn next(&mut self) -> Option<Self::Item> {
        match self.memchr.next() {
            Some(index) => {
                let start = self.offset;
                self.offset = index + 1;
                Some(&self.haystack[start..index])
            }
            None if self.offset < self.haystack.len() => {
                let start = self.offset;
                self.offset = self.haystack.len();
                Some(&self.haystack[start..])
            }
            None => None,
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        // Every item consumes at least one byte, including empty items between delimiters.
        let remaining = self.haystack.len() - self.offset;
        (usize::from(remaining > 0), Some(remaining))
    }
}

#[cfg(test)]
mod tests {
    use super::MemchrSplitter;

    #[test]
    fn size_hint_bounds_adjacent_delimiters() {
        let splitter = MemchrSplitter::split("....", b'.');
        let (_, Some(upper)) = splitter.size_hint() else {
            panic!("the upper bound should be known");
        };
        assert!(splitter.count() <= upper);
    }
}
