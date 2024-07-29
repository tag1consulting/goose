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

#[cfg(test)]
mod test {
    use crate::report::common::OrEmpty;

    #[test]
    pub fn format() {
        assert_eq!("1.23", format!("{:.2}", OrEmpty(Some(1.23456))));
        assert_eq!("1", format!("{:.0}", OrEmpty(Some(1.23456))));
        assert_eq!("", format!("{:.2}", OrEmpty::<f32>(None)));
    }
}
