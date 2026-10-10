use std::fmt::{Display, Formatter};
use std::str::Chars;

use crate::{Pep508Error, Pep508ErrorSource, Pep508Url};

/// A [`Cursor`] over a string.
#[derive(Debug, Clone)]
pub(crate) struct Cursor<'a> {
    input: &'a str,
    chars: Chars<'a>,
    pos: usize,
}

impl<'a> Cursor<'a> {
    /// Convert from `&str`.
    pub(crate) fn new(input: &'a str) -> Self {
        Self {
            input,
            chars: input.chars(),
            pos: 0,
        }
    }

    /// Returns a new cursor starting at the given position.
    pub(crate) fn at(self, pos: usize) -> Self {
        Self {
            input: self.input,
            chars: self.input[pos..].chars(),
            pos,
        }
    }

    /// Returns the current byte position of the cursor.
    pub(crate) fn pos(&self) -> usize {
        self.pos
    }

    /// Returns a slice over the input string.
    pub(crate) fn slice(&self, start: usize, len: usize) -> &str {
        &self.input[start..start + len]
    }

    /// Peeks the next character and position from the input stream without consuming it.
    pub(crate) fn peek(&self) -> Option<(usize, char)> {
        self.chars.clone().next().map(|char| (self.pos, char))
    }

    /// Peeks the next character from the input stream without consuming it.
    pub(crate) fn peek_char(&self) -> Option<char> {
        self.chars.clone().next()
    }

    /// Eats the next character from the input stream if it matches the given token.
    pub(crate) fn eat_char(&mut self, token: char) -> Option<usize> {
        let (start_pos, peek_char) = self.peek()?;
        if peek_char == token {
            self.next();
            Some(start_pos)
        } else {
            None
        }
    }

    /// Consumes whitespace from the cursor.
    pub(crate) fn eat_whitespace(&mut self) {
        while let Some(char) = self.peek_char() {
            if char.is_whitespace() {
                self.next();
            } else {
                return;
            }
        }
    }

    /// Returns the next character and position from the input stream and consumes it.
    pub(crate) fn next(&mut self) -> Option<(usize, char)> {
        let pos = self.pos;
        let char = self.chars.next()?;
        self.pos += char.len_utf8();
        Some((pos, char))
    }

    /// Peeks over the cursor as long as the condition is met, without consuming it.
    pub(crate) fn peek_while(&mut self, condition: impl Fn(char) -> bool) -> (usize, usize) {
        let peeker = self.chars.clone();
        let start = self.pos();
        let len = peeker
            .take_while(|c| condition(*c))
            .map(char::len_utf8)
            .sum();
        (start, len)
    }

    /// Consumes characters from the cursor as long as the condition is met.
    pub(crate) fn take_while(&mut self, condition: impl Fn(char) -> bool) -> (usize, usize) {
        let start = self.pos();
        let mut len = 0;
        while let Some(char) = self.peek_char() {
            if !condition(char) {
                break;
            }

            self.next();
            len += char.len_utf8();
        }
        (start, len)
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

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;

    use crate::{Pep508Error, VerbatimUrl};

    use super::Cursor;

    #[test]
    fn utf8_byte_offsets() {
        let input = "aé中🦀z";
        let mut cursor = Cursor::new(input);

        for (start, character, end) in [
            (0, 'a', 1),
            (1, 'é', 3),
            (3, '中', 6),
            (6, '🦀', 10),
            (10, 'z', 11),
        ] {
            assert_eq!(cursor.pos(), start);
            assert_eq!(cursor.peek(), Some((start, character)));
            assert_eq!(cursor.peek_char(), Some(character));
            assert_eq!(cursor.pos(), start);
            assert_eq!(cursor.next(), Some((start, character)));
            assert_eq!(cursor.pos(), end);
            assert_eq!(cursor.slice(start, end - start), character.to_string());
        }

        assert_eq!(cursor.peek(), None);
        assert_eq!(cursor.peek_char(), None);
        assert_eq!(cursor.next(), None);
        assert_eq!(cursor.pos(), input.len());
        assert_eq!(cursor.slice(1, 9), "é中🦀");

        let mut cursor = cursor.at(3);
        assert_eq!(cursor.pos(), 3);
        assert_eq!(cursor.next(), Some((3, '中')));
        assert_eq!(cursor.pos(), 6);
        assert_eq!(cursor.to_string(), input);
    }

    #[test]
    fn utf8_while_spans() {
        for (input, prefix, span, expected, next) in [
            ("!abc?", '!', (1, 3), "abc", Some('?')),
            ("!é?", '!', (1, 2), "é", Some('?')),
            ("!中?", '!', (1, 3), "中", Some('?')),
            ("!🦀?", '!', (1, 4), "🦀", Some('?')),
            ("é中🦀?", 'é', (2, 7), "中🦀", Some('?')),
            ("!?", '!', (1, 0), "", Some('?')),
            ("!é中🦀", '!', (1, 9), "é中🦀", None),
        ] {
            let mut cursor = Cursor::new(input);
            assert_eq!(cursor.next(), Some((0, prefix)));
            let original_peek = cursor.peek();

            assert_eq!(cursor.peek_while(|character| character != '?'), span);
            assert_eq!(cursor.slice(span.0, span.1), expected);
            assert_eq!(cursor.pos(), span.0);
            assert_eq!(cursor.peek(), original_peek);
            assert_eq!(cursor.take_while(|_| false), (span.0, 0));
            assert_eq!(cursor.peek(), original_peek);

            assert_eq!(cursor.take_while(|character| character != '?'), span);
            assert_eq!(cursor.pos(), span.0 + span.1);
            assert_eq!(cursor.peek_char(), next);
        }
    }

    #[test]
    fn expected_character_spans() -> Result<(), Pep508Error> {
        let mut diagnostics = Vec::new();
        for (input, character, len) in [
            ("éx", 'x', 1),
            ("éα", 'α', 2),
            ("é中", '中', 3),
            ("é🦀", '🦀', 4),
        ] {
            let mut cursor = Cursor::new(input).at(2);
            let error = cursor
                .next_expect_char::<VerbatimUrl>(')', 0)
                .expect_err("the next character should not match");
            assert_eq!((error.start, error.len), (2, len));
            assert_eq!(error.input, input);
            assert_eq!(
                error.message.to_string(),
                format!("Expected `)`, found `{character}`")
            );
            assert_eq!(cursor.pos(), input.len());
            diagnostics.push(error.to_string());
        }
        assert_snapshot!(diagnostics.join("\n\n"), @"
        Expected `)`, found `x`
        éx
         ^

        Expected `)`, found `α`
        éα
         ^

        Expected `)`, found `中`
        é中
         ^^

        Expected `)`, found `🦀`
        é🦀
         ^^
        ");

        let mut cursor = Cursor::new("é)").at(2);
        cursor.next_expect_char::<VerbatimUrl>(')', 0)?;
        assert_eq!(cursor.pos(), 3);

        let mut cursor = Cursor::new("é(").at(3);
        let error = cursor
            .next_expect_char::<VerbatimUrl>(')', 2)
            .expect_err("the cursor should be at the end of the input");
        assert_eq!((error.start, error.len), (2, 1));
        assert_snapshot!(error, @"
        Expected ')', found end of dependency specification
        é(
         ^
        ");

        Ok(())
    }
}
