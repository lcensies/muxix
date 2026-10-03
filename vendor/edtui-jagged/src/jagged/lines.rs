use crate::{Index2, Jagged};

type Lines = Jagged<char>;

impl<T: AsRef<str>> From<T> for Jagged<char> {
    fn from(value: T) -> Self {
        let mut data: Vec<Vec<char>> = Vec::new();
        for line in value.as_ref().lines() {
            data.push(line.chars().collect());
        }

        if let Some(last) = value.as_ref().chars().last() {
            if last == '\n' {
                data.push(Vec::new());
            }
        }

        Self { data }
    }
}

impl From<Jagged<char>> for String {
    fn from(value: Jagged<char>) -> String {
        value.flatten(&Some('\n')).into_iter().collect()
    }
}

impl Lines {
    /// Returns the data as a single String, with lines joined by newlines.
    pub fn to_string(&self) -> String {
        self.data
            .iter()
            .map(|line| line.iter().collect::<String>())
            .collect::<Vec<String>>()
            .join("\n")
    }

    /// Finds the index of the matching (closing or opening) bracket from a given starting point.
    #[must_use]
    #[deprecated(
        since = "0.1.9",
        note = "Line specifics should not be part of this library"
    )]
    pub fn find_matching_bracket(&self, index: Index2) -> Option<Index2> {
        let &opening_bracket = self.get(index)?;

        let (closing_bracket, reverse) = match opening_bracket {
            '{' => ('}', false),
            '}' => ('{', true),
            '(' => (')', false),
            ')' => ('(', true),
            '[' => (']', false),
            ']' => ('[', true),
            _ => return None,
        };

        let mut counter = 0;

        let iter: Box<dyn Iterator<Item = (Option<&char>, Index2)>> = if reverse {
            Box::new(self.iter().from(index).rev().skip(1))
        } else {
            Box::new(self.iter().from(index).skip(1))
        };

        for (value, index) in iter {
            let Some(&value) = value else { continue };

            if value == opening_bracket {
                counter += 1;
            }

            if value == closing_bracket {
                if counter == 0 {
                    return Some(index);
                }
                counter -= 1;
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    #![allow(deprecated)]
    use super::*;

    #[test]
    fn test_find_closing_bracket() {
        let cursor = Index2::new(0, 0);
        let lines = Jagged::from("{ab\n{{}}c}d");

        let closing_bracket = lines.find_matching_bracket(cursor);
        assert_eq!(closing_bracket, Some(Index2::new(1, 5)));

        let cursor = Index2::new(1, 5);
        let closing_bracket = lines.find_matching_bracket(cursor);
        assert_eq!(closing_bracket, Some(Index2::new(0, 0)));
    }
}
