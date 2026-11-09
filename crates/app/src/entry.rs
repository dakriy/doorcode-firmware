//! The entry being typed at the keypad: digits and `*`, ended by the submit
//! key, forgotten after a while without a key.

use protocol::Config;

#[derive(Default)]
pub(crate) struct Entry {
    keys: String,
    /// When the last key was taken.
    last_key_at: u64,
}

impl Entry {
    /// The keys typed so far.
    pub(crate) fn keys(&self) -> &str {
        &self.keys
    }

    /// When the entry is forgotten unless another key comes; `None` while
    /// nothing is typed.
    pub(crate) fn clear_at(&self, config: &Config) -> Option<u64> {
        if self.keys.is_empty() {
            return None;
        }
        Some(self.last_key_at + config.keypad_idle_clear_ms)
    }

    /// Forgets the keys typed so far.
    pub(crate) fn clear(&mut self) {
        self.keys.clear();
    }

    /// Takes `key`, and returns the entry when the key submits one whose
    /// code is long enough to ask about. The submit key is not part of the
    /// entry; other keys that are neither digits nor `*` are not taken, and
    /// neither is any key once the entry is as long as it may be.
    pub(crate) fn press(&mut self, key: char, now_ms: u64, config: &Config) -> Option<String> {
        if config.submit_key.chars().eq([key]) {
            let entry = std::mem::take(&mut self.keys);
            return long_enough(&entry, config).then_some(entry);
        }
        let takes = key.is_ascii_digit() || key == '*';
        if takes && (self.keys.len() as u64) < config.max_code_length {
            self.keys.push(key);
            self.last_key_at = now_ms;
        }
        None
    }
}

/// Whether the code, the digits before any `*`, is at least as long as the
/// shortest code.
fn long_enough(entry: &str, config: &Config) -> bool {
    let code = entry.split('*').next().unwrap_or_default();
    code.len() as u64 >= config.min_code_length
}
