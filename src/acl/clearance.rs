// bole-fo2
use serde::{Deserialize, Serialize};

// bole-fo2
bitflags::bitflags! {
    /// Orthogonal to the lattice position: a clearance can grant read, write,
    /// or both, independent of which label it is for.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Capability: u8 {
        const READ = 0b01;
        const WRITE = 0b10;
    }
}

#[cfg(test)]
mod tests {
    use super::Capability;

    #[test]
    fn capability_bit_ops() {
        let rw = Capability::READ | Capability::WRITE;
        assert!(rw.contains(Capability::READ));
        assert!(rw.contains(Capability::WRITE));
        assert!(Capability::READ.contains(Capability::READ));
        assert!(!Capability::READ.contains(Capability::WRITE));
        assert!(!Capability::WRITE.contains(Capability::READ));
    }
}
