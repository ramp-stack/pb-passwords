//! Shared, redacted storage for temporary password and clipboard values.
use std::{cell::RefCell, fmt, rc::Rc};
use zeroize::{Zeroize, Zeroizing};

#[derive(Clone)]
pub struct Secret(pub Rc<RefCell<Zeroizing<String>>>);

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([redacted])")
    }
}

impl Default for Secret {
    fn default() -> Self {
        Self::new("")
    }
}

impl Secret {
    pub fn new(value: &str) -> Self {
        Self(Rc::new(RefCell::new(Zeroizing::new(value.to_owned()))))
    }

    pub fn clear(&self) {
        self.0.borrow_mut().zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clearing_erases_shared_values_and_debug_output_is_redacted() {
        let value = Secret::new("private test value");
        let clone = value.clone();
        assert_eq!(format!("{clone:?}"), "Secret([redacted])");
        value.clear();
        assert!(clone.0.borrow().is_empty());
    }
}
