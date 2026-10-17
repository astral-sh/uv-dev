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
        // We know we'll return at least one item if there's remaining text.
        let min = usize::from(self.offset < self.haystack.len());

        // Each item consumes at least one byte, even when delimiters are adjacent.
        let max = self.haystack.len() - self.offset;

        (min, Some(max))
    }
}

#[cfg(test)]
mod tests {
    use super::MemchrSplitter;

    #[test]
    fn size_hint_bounds_remaining_items() {
        for (haystack, delimiter) in [
            ("", b'.'),
            (".", b'.'),
            ("..", b'.'),
            ("...", b'.'),
            ("....", b'.'),
            ("a", b'.'),
            ("a.", b'.'),
            (".a", b'.'),
            ("a..b", b'.'),
            ("..a..b..", b'.'),
            ("py2.py3", b'.'),
            ("manylinux_2_17_x86_64.manylinux2014_x86_64", b'.'),
            ("é.🦀..終.", b'.'),
            ("é🦀終", b'.'),
            ("---", b'-'),
            ("a--b-", b'-'),
            ("\0a\0\0", b'\0'),
        ] {
            let expected: Vec<_> = haystack.split_terminator(char::from(delimiter)).collect();
            let mut splitter = MemchrSplitter::split(haystack, delimiter);

            for consumed in 0..=expected.len() {
                let remaining = expected.len() - consumed;
                let (lower, upper) = splitter.size_hint();
                assert!(
                    lower <= remaining && upper.is_none_or(|upper| remaining <= upper),
                    "{haystack:?}, delimiter {delimiter:?}, consumed {consumed}: \
                     size hint ({lower}, {upper:?}) does not contain {remaining} remaining items"
                );
                assert_eq!(
                    splitter.next(),
                    expected.get(consumed).copied(),
                    "{haystack:?}, delimiter {delimiter:?}, consumed {consumed}"
                );
            }

            assert_eq!(splitter.size_hint(), (0, Some(0)));
            assert_eq!(splitter.next(), None);
        }
    }
}
