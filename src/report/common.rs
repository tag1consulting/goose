use num_format::{Locale, ToFormattedString};
use std::fmt::{Display, Formatter};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrEmpty<T>(pub Option<T>);

impl<T: Display> Display for OrEmpty<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            Some(value) => value.fmt(f),
            None => f.write_str(""),
        }
    }
}

pub struct FormattedNumber<T>(pub T)
where
    T: ToFormattedString;

impl<T> Display for FormattedNumber<T>
where
    T: ToFormattedString,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0.to_formatted_string(&Locale::en))
    }
}

#[cfg(test)]
mod test {
    use crate::report::common::{FormattedNumber, OrEmpty};

    #[test]
    pub fn format_or_empty() {
        assert_eq!("1.23", format!("{:.2}", OrEmpty(Some(1.23456))));
        assert_eq!("1", format!("{:.0}", OrEmpty(Some(1.23456))));
        assert_eq!("", format!("{:.2}", OrEmpty::<f32>(None)));
    }

    #[test]
    pub fn format_number_format() {
        assert_eq!("1", format!("{:.2}", FormattedNumber(1)));
        assert_eq!("1,000", format!("{:.2}", FormattedNumber(1000)));
        assert_eq!("1,000,000", format!("{:.2}", FormattedNumber(1000000)));
    }
}
