use zeroize::{Zeroize, ZeroizeOnDrop};

/// A credential blob.
///
/// Security contract: this is the only place raw token bytes are held. It is
/// never serialized, never printed in the clear, and is wiped on drop.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct Secret(Vec<u8>);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Secret([redacted; {} bytes])", self.0.len())
    }
}

impl Secret {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Borrow the raw bytes. Named `expose` to make every call site auditable.
    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_original_bytes() {
        let s = Secret::new(b"abc".to_vec());
        assert_eq!(s.expose(), b"abc");
        assert_eq!(s.len(), 3);
        assert!(!s.is_empty());
    }

    #[test]
    fn empty_secret_boundary() {
        let s = Secret::new(Vec::new());
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
        assert_eq!(format!("{s:?}"), "Secret([redacted; 0 bytes])");
    }

    #[test]
    fn debug_never_reveals_contents() {
        let s = Secret::new(b"super-secret-token".to_vec());
        let shown = format!("{s:?}");
        assert!(
            !shown.contains("super-secret-token"),
            "leaked literal: {shown}"
        );
        assert!(
            shown.contains("redacted"),
            "Debug must be redacted, got: {shown}"
        );
    }

    #[test]
    fn zeroize_clears_buffer() {
        let mut s = Secret::new(b"abc".to_vec());
        s.zeroize();
        assert!(s.is_empty());
    }
}
