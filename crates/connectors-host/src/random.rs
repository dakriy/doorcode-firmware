use connectors::Random;

/// Entropy from the operating system.
#[derive(Debug, Default)]
pub struct OsRandom;

impl Random for OsRandom {
    fn bytes(&self, count: usize) -> Vec<u8> {
        let mut bytes = vec![0; count];
        getrandom::fill(&mut bytes).expect("the operating system provides entropy");
        bytes
    }
}
