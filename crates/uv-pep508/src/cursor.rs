use std::fmt::{Display, Formatter};
use std::str::Chars;

use crate::{Pep508Error, Pep508ErrorSource, Pep508Url};

/// A [`Cursor`] over a string.
#[derive(Debug, Clone)]
pub(crate) struct Cursor<'a> {
    input: &'a str,
    chars: Chars<'a>,
}

impl<'a> Cursor<'a> {
    /// Convert from `&str`.
    pub(crate) fn new(input: &'a str) -> Self {
        Self {
            input,
            chars: input.chars(),
        }
    }

    /// Returns a new cursor starting at the given position.
    pub(crate) fn at(self, pos: usize) -> Self {
        Self {
            input: self.input,
            chars: self.input[pos..].chars(),
        }
    }

    /// Returns the current byte position of the cursor.
    pub(crate) fn pos(&self) -> usize {
        self.input.len() - self.remaining().len()
    }

    /// Returns the unconsumed input.
    pub(crate) fn remaining(&self) -> &'a str {
        self.chars.as_str()
    }

    /// Returns a slice over the input string.
    pub(crate) fn slice(&self, start: usize, len: usize) -> &str {
        &self.input[start..start + len]
    }

    /// Peeks the next character and position from the input stream without consuming it.
    pub(crate) fn peek(&self) -> Option<(usize, char)> {
        self.peek_char().map(|char| (self.pos(), char))
    }

    /// Peeks the next character from the input stream without consuming it.
    pub(crate) fn peek_char(&self) -> Option<char> {
        self.chars.clone().next()
    }

    /// Eats the next character from the input stream if it matches the given token.
    pub(crate) fn eat_char(&mut self, token: char) -> Option<usize> {
        let remaining = self.remaining().strip_prefix(token)?;
        let start = self.pos();
        self.advance_to(remaining);
        Some(start)
    }

    /// Consumes whitespace from the cursor.
    pub(crate) fn eat_whitespace(&mut self) {
        self.take_while(char::is_whitespace);
    }

    /// Returns the next character and position from the input stream and consumes it.
    pub(crate) fn next(&mut self) -> Option<(usize, char)> {
        let pos = self.pos();
        let char = self.chars.next()?;
        Some((pos, char))
    }

    /// Consume a keyword followed by a delimiter or the end of input.
    pub(crate) fn eat_keyword(
        &mut self,
        keyword: &str,
        is_delimiter: impl FnOnce(char) -> bool,
    ) -> bool {
        let Some(remaining) = self.remaining().strip_prefix(keyword) else {
            return false;
        };
        if remaining.chars().next().is_some_and(|c| !is_delimiter(c)) {
            return false;
        }

        self.advance_to(remaining);
        true
    }

    /// Consumes characters from the cursor as long as the condition is met, returning the start
    /// position and length in bytes.
    pub(crate) fn take_while(&mut self, condition: impl Fn(char) -> bool) -> (usize, usize) {
        let start = self.pos();
        self.advance_to(self.remaining().trim_start_matches(condition));
        (start, self.pos() - start)
    }

    /// Advance to a suffix of the current unconsumed input.
    fn advance_to(&mut self, remaining: &'a str) {
        self.chars = remaining.chars();
    }

    /// Consumes characters from the cursor, raising an error if it doesn't match the given token.
    pub(crate) fn next_expect_char<T: Pep508Url>(
        &mut self,
        expected: char,
        span_start: usize,
    ) -> Result<(), Pep508Error<T>> {
        match self.next() {
            None => Err(Pep508Error {
                message: Pep508ErrorSource::String(format!(
                    "Expected '{expected}', found end of dependency specification"
                )),
                start: span_start,
                len: 1,
                input: self.to_string(),
            }),
            Some((_, value)) if value == expected => Ok(()),
            Some((pos, other)) => Err(Pep508Error {
                message: Pep508ErrorSource::String(format!(
                    "Expected `{expected}`, found `{other}`"
                )),
                start: pos,
                len: other.len_utf8(),
                input: self.to_string(),
            }),
        }
    }
}

impl Display for Cursor<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.input)
    }
}
