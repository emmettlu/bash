#[derive(Default)]
pub(super) struct PlusMinusFlag {
    value: Option<bool>,
}

impl PlusMinusFlag {
    pub(super) const fn is_some(&self) -> bool {
        self.value.is_some()
    }

    pub(super) const fn to_bool(&self) -> Option<bool> {
        self.value
    }

    pub(super) fn set(&mut self, enabled: bool) {
        self.value = Some(enabled);
    }
}

#[cfg(test)]
mod tests {
    use super::PlusMinusFlag;

    #[test]
    fn last_plus_minus_flag_wins() {
        let mut flag = PlusMinusFlag::default();
        flag.set(true);
        flag.set(false);
        assert_eq!(flag.to_bool(), Some(false));

        flag.set(true);
        assert_eq!(flag.to_bool(), Some(true));
    }
}
