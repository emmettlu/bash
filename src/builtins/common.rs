#[derive(Default)]
pub(super) struct PlusMinusFlag {
    enable: bool,
    disable: bool,
}

impl PlusMinusFlag {
    pub(super) const fn is_some(&self) -> bool {
        self.enable || self.disable
    }

    pub(super) const fn to_bool(&self) -> Option<bool> {
        match (self.enable, self.disable) {
            (true, false) => Some(true),
            (false, true) => Some(false),
            _ => None,
        }
    }

    pub(super) fn set(&mut self, enabled: bool) {
        if enabled {
            self.enable = true;
        } else {
            self.disable = true;
        }
    }
}
